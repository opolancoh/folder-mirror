use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use chrono::Local;

use folder_mirror::app;
use folder_mirror::features::plan::{SyncPlan, EXIT_ABORTED, EXIT_DIFFERENCES};
use folder_mirror::features::report::{self, ReportFormat, ReportPaths};
use folder_mirror::shared::config::{ConfigFile, Profile, Settings};

use super::args::{Cli, OptionArgs, TargetArgs};
use super::output::{self, RunMode};
use super::progress::ConsoleProgress;

const DEFAULT_CONFIG: &str = "mirror.toml";

static CANCEL: AtomicBool = AtomicBool::new(false);

/// Runs the tool and returns the process exit code.
pub fn run(args: Cli) -> i32 {
    let nothing_given = args.target.source.is_none()
        && args.target.destination.is_none()
        && args.target.profile.is_none()
        && args.plan.is_none();
    let result = if nothing_given {
        output::print_overview();
        Ok(0)
    } else {
        mirror(args)
    };
    result.unwrap_or_else(|err| {
        output::print_fatal(&err);
        EXIT_ABORTED
    })
}

/// One run: analyze (or load a plan), show the summary, ask, apply, report.
fn mirror(args: Cli) -> Result<i32> {
    // The report name uses the time the run started, with its time zone.
    let started = Local::now();
    let mode = if args.dry_run {
        RunMode::DryRun
    } else if args.confirm {
        RunMode::Ask
    } else {
        RunMode::Automatic
    };
    let (settings, mut plan) = match &args.plan {
        Some(path) => {
            let plan = app::load_plan(path)?;
            let settings = build_settings(&args.target, &args.options, Some(&plan))?;
            app::configure_threads(&settings)?;
            output::print_settings(&settings, mode);
            (settings, plan)
        }
        None => {
            let settings = build_settings(&args.target, &args.options, None)?;
            app::configure_threads(&settings)?;
            output::print_settings(&settings, mode);
            let plan = run_analysis(&settings)?;
            (settings, plan)
        }
    };
    output::print_analysis(&plan, &settings);

    // Step 1: save the analysis now, so it is kept even if the sync is interrupted.
    let format = args.output.format.unwrap_or_default();
    let dir = args
        .output
        .output_dir
        .clone()
        .or_else(|| settings.report_dir.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    let paths = report::report_paths(&dir, format, started)
        .with_context(|| format!("cannot create report files in {}", dir.display()))?;
    save(&plan, &paths, format, plan.exit_code())?;

    // Step 2: decide, apply, and update the same report.
    let not_executed = if !plan.blockers.is_empty() {
        Some(("blocked by safety checks", EXIT_ABORTED))
    } else if !plan.has_pending_actions() {
        Some(("nothing to do, the destination is already identical", plan.exit_code()))
    } else if mode == RunMode::DryRun {
        Some(("dry run (--dry-run)", plan.exit_code()))
    } else if mode == RunMode::Ask && !io::stdin().is_terminal() {
        output::print_not_interactive();
        Some(("no terminal to ask for confirmation", EXIT_ABORTED))
    } else if mode == RunMode::Ask && !confirm()? {
        output::print_declined();
        Some(("declined by the user", EXIT_DIFFERENCES))
    } else {
        None
    };
    if let Some((reason, code)) = not_executed {
        plan.not_executed = Some(reason.to_string());
        return finish(&plan, &paths, format, code);
    }

    let confirmation = match mode {
        RunMode::Ask => "confirmed by the user",
        _ => "automatic (--confirm false)",
    };
    install_ctrlc_handler()?;
    {
        let progress = ConsoleProgress::new();
        app::sync(&mut plan, &settings, &progress, &CANCEL, confirmation)?;
    }
    output::print_sync_result(&plan, &settings);
    finish(&plan, &paths, format, plan.exit_code())
}

fn run_analysis(settings: &Settings) -> Result<SyncPlan> {
    let progress = ConsoleProgress::new();
    Ok(app::analyze(settings, &progress)?)
}

/// Writes the result file (JSON or CSV) and the HTML report.
fn save(plan: &SyncPlan, paths: &ReportPaths, format: ReportFormat, code: i32) -> Result<()> {
    report::write_report(plan, &paths.result, format)
        .with_context(|| format!("cannot write the result file {}", paths.result.display()))?;
    report::write_html(plan, &paths.html, code)
        .with_context(|| format!("cannot write the HTML report {}", paths.html.display()))?;
    Ok(())
}

/// Final save of the report, then prints where it is.
fn finish(plan: &SyncPlan, paths: &ReportPaths, format: ReportFormat, code: i32) -> Result<i32> {
    save(plan, paths, format, code)?;
    output::print_footer(&paths.result, &paths.html, code);
    Ok(code)
}

/// Merges defaults, the profile, the plan file and the command line (in that order).
fn build_settings(target: &TargetArgs, options: &OptionArgs, plan: Option<&SyncPlan>) -> Result<Settings> {
    let profile = load_profile(target)?;
    let from_profile = |pick: fn(&Profile) -> Option<PathBuf>| profile.as_ref().and_then(pick);

    let (source, destination) = match plan {
        Some(plan) => (PathBuf::from(&plan.source), PathBuf::from(&plan.destination)),
        None => (
            target
                .source
                .clone()
                .or_else(|| from_profile(|p| p.source.clone()))
                .context("missing source folder: use --source or a profile with `source`")?,
            target
                .destination
                .clone()
                .or_else(|| from_profile(|p| p.destination.clone()))
                .context("missing destination folder: use --destination or a profile with `destination`")?,
        ),
    };

    let mut settings = Settings::new(source, destination);
    if let Some(plan) = plan {
        settings.delete_mode = plan.delete_mode;
    }
    if let Some(profile) = &profile {
        settings.apply_profile(profile)?;
    }
    options.apply(&mut settings)?;
    if let Some(plan) = plan {
        // The comparison already happened; keep the mode the plan was made with.
        settings.mode = plan.mode;
    }
    settings.validate()?;
    Ok(settings)
}

fn load_profile(target: &TargetArgs) -> Result<Option<Profile>> {
    let Some(name) = &target.profile else {
        return Ok(None);
    };
    let path = target.config.clone().unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    let config = ConfigFile::load(&path)?;
    let mut profile = config.profile(name)?.clone();
    // Relative folders in the profile are relative to the config file, not the current folder.
    let base = path.parent().unwrap_or(Path::new(""));
    for folder in [&mut profile.source, &mut profile.destination, &mut profile.output_dir].into_iter().flatten() {
        if folder.is_relative() {
            *folder = base.join(&*folder);
        }
    }
    Ok(Some(profile))
}

fn confirm() -> Result<bool> {
    print!("\nProceed with sync? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

/// First Ctrl+C: finish the files in progress and stop. Second Ctrl+C: quit immediately.
fn install_ctrlc_handler() -> Result<()> {
    ctrlc::set_handler(|| {
        if CANCEL.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        eprintln!("\nStopping after the files in progress... press Ctrl+C again to quit immediately.");
    })
    .context("cannot install the Ctrl+C handler")
}
