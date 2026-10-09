use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};

use folder_mirror::app;
use folder_mirror::features::plan::{SyncPlan, EXIT_ABORTED, EXIT_DIFFERENCES};
use folder_mirror::features::report::{self, ReportFormat};
use folder_mirror::shared::config::{ConfigFile, Profile, Settings};

use super::args::{AnalyzeArgs, Cli, Command, OptionArgs, OutputArgs, SyncArgs, TargetArgs};
use super::output;
use super::progress::ConsoleProgress;

const DEFAULT_CONFIG: &str = "mirror.toml";

static CANCEL: AtomicBool = AtomicBool::new(false);

/// Runs the command and returns the process exit code.
pub fn run(cli: Cli) -> i32 {
    let result = match cli.command {
        None => {
            output::print_overview();
            Ok(0)
        }
        Some(Command::Analyze(args)) => analyze(args),
        Some(Command::Sync(args)) => sync(args),
    };
    result.unwrap_or_else(|err| {
        output::print_fatal(&err);
        EXIT_ABORTED
    })
}

fn analyze(args: AnalyzeArgs) -> Result<i32> {
    let settings = build_settings(&args.target, &args.options, None)?;
    app::configure_threads(&settings)?;
    output::print_settings(&settings, "analyze");
    let plan = run_analysis(&settings)?;
    output::print_analysis(&plan, &settings);
    finish(&plan, &args.output, plan.exit_code())
}

fn sync(args: SyncArgs) -> Result<i32> {
    let (settings, mut plan) = match &args.plan {
        Some(path) => {
            let plan = app::load_plan(path)?;
            let settings = build_settings(&args.target, &args.options, Some(&plan))?;
            app::configure_threads(&settings)?;
            output::print_settings(&settings, "sync");
            (settings, plan)
        }
        None => {
            let settings = build_settings(&args.target, &args.options, None)?;
            app::configure_threads(&settings)?;
            output::print_settings(&settings, "sync");
            let plan = run_analysis(&settings)?;
            (settings, plan)
        }
    };
    output::print_analysis(&plan, &settings);

    if !plan.blockers.is_empty() {
        return finish(&plan, &args.output, EXIT_ABORTED);
    }
    if !plan.has_pending_actions() {
        output::print_in_sync();
        return finish(&plan, &args.output, plan.exit_code());
    }
    if !args.yes {
        if !io::stdin().is_terminal() {
            output::print_not_interactive();
            return finish(&plan, &args.output, EXIT_ABORTED);
        }
        if !confirm()? {
            output::print_declined();
            return finish(&plan, &args.output, EXIT_DIFFERENCES);
        }
    }

    install_ctrlc_handler()?;
    {
        let progress = ConsoleProgress::new();
        app::sync(&mut plan, &settings, &progress, &CANCEL)?;
    }
    output::print_sync_result(&plan, &settings);
    finish(&plan, &args.output, plan.exit_code())
}

fn run_analysis(settings: &Settings) -> Result<SyncPlan> {
    let progress = ConsoleProgress::new();
    Ok(app::analyze(settings, &progress)?)
}

/// Writes the result file (JSON or CSV) plus an HTML report with the same name, and prints where.
fn finish(plan: &SyncPlan, args: &OutputArgs, code: i32) -> Result<i32> {
    let format = args
        .format
        .or_else(|| args.output.as_deref().and_then(ReportFormat::from_path))
        .unwrap_or_default();
    let mut path = args.output.clone().unwrap_or_else(|| report::default_report_path(format));
    let html = path.with_extension("html");
    if path == html {
        path = path.with_extension(format.extension());
    }
    report::write_report(plan, &path, format)
        .with_context(|| format!("cannot write the result file {}", path.display()))?;
    report::write_html(plan, &html, code)
        .with_context(|| format!("cannot write the HTML report {}", html.display()))?;
    output::print_footer(&path, &html, code);
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
    for folder in [&mut profile.source, &mut profile.destination].into_iter().flatten() {
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
