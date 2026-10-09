//! Console output: summaries, warnings and problems.

use std::collections::BTreeMap;
use std::path::Path;

use console::{style, StyledObject};

use folder_mirror::features::plan::{exit_code_meaning, Counter, SyncPlan};
use folder_mirror::shared::config::{CompareMode, DeleteMode, Settings};
use folder_mirror::shared::error::Problem;
use folder_mirror::shared::format::{format_bytes, format_count};

/// Shown when the tool runs without a command.
pub fn print_overview() {
    println!("{}", style("mirror · keeps a mirror folder identical to its source folder (one-way)").bold());
    println!();
    println!("{}", style("How it works").bold());
    println!("  1. analyze  scans both folders and lists what differs. Changes nothing.");
    println!("  2. sync     shows the same summary, asks for confirmation, then copies new files,");
    println!("              overwrites edited files and removes deleted files from the mirror.");
    println!("  The source is always right. Each run writes a result file with every file's full");
    println!("  path and action. Failed files never stop the sync; they are listed at the end.");
    println!();
    println!("{}", style("Default settings").bold());
    print_setting_lines(&Settings::new("", ""));
    println!();
    println!("{}", style("Examples").bold());
    println!("  mirror analyze -s \"C:\\source\" -d \"D:\\mirror\"");
    println!("  mirror sync    -s \"C:\\source\" -d \"D:\\mirror\"");
    println!("  mirror sync --help        all options");
}

/// The settings a run will use; `*` marks values changed from the default.
/// How this run will work, from the effective settings (defaults or given by the user).
pub fn print_settings(settings: &Settings, command: &str) {
    println!();
    let what = if command == "sync" {
        "makes the destination identical to the source, after your confirmation"
    } else {
        "compares both folders and reports the differences; nothing is changed"
    };
    println!("{} {command} · {what}", style("mirror").bold());
    let full = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).display().to_string();
    line("Source", full(&settings.source));
    line("Destination", full(&settings.destination));
    print_setting_lines(settings);
    println!();
}

fn line(label: &str, value: String) {
    println!("  {:<18} {value}", format!("{label}:"));
}

fn print_setting_lines(settings: &Settings) {
    line(
        "Match files",
        format!(
            "{} · {}; times within {} s count as equal",
            settings.mode,
            mode_help(settings.mode),
            settings.mtime_tolerance.as_secs_f64()
        ),
    );
    line("Removed files", format!("{} · {}", settings.delete_mode, delete_help(settings.delete_mode)));
    line(
        "Overwritten files",
        if settings.backup_overwritten {
            "the old version is kept, same place as removed files".into()
        } else {
            "replaced, the old version is not kept".into()
        },
    );
    if settings.delete_mode != DeleteMode::Permanent {
        line("Trash folder", format!("keeps the last {} runs in .mirror-trash", settings.trash_keep));
    }
    line(
        "Safety",
        format!(
            "the sync is blocked if more than {}% of the mirror would be deleted, or if space is short",
            settings.max_delete_percent
        ),
    );
    line(
        "Locked files",
        format!("tried {} times, waiting {} s and then longer", settings.retries, settings.retry_delay.as_secs_f64()),
    );
    line(
        "On errors",
        if settings.stop_on_error {
            "stops at the first failed file".into()
        } else {
            format!(
                "continues; problems listed with paths (up to {}, counts above that)",
                settings.max_errors_shown
            )
        },
    );
    line(
        "Verify copies",
        if settings.verify { "yes, every copied file is hashed".into() } else { "no".into() },
    );
    let mut excluded = Vec::new();
    if settings.default_excludes {
        excluded.push("OS files (Thumbs.db, .DS_Store, $RECYCLE.BIN...)".to_string());
    }
    excluded.extend(settings.excludes.iter().cloned());
    line("Excluded", if excluded.is_empty() { "nothing".into() } else { excluded.join(", ") });
}

fn mode_help(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::Name => "only added and removed files, edits are not detected",
        CompareMode::Metadata => "size and modification time",
        CompareMode::Smart => "size and time; a full hash confirms same-size files with a new time",
        CompareMode::QuickHash => "size and a hash of 3 sampled blocks",
        CompareMode::FullHash => "size and a hash of the whole file (exact, slowest)",
    }
}

fn delete_help(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::RecycleBin => "OS Recycle Bin; trash folder where none exists (network drives)",
        DeleteMode::TrashFolder => "moved to <destination>\\.mirror-trash\\<date>",
        DeleteMode::Permanent => "deleted for good",
    }
}

/// Summary of what the sync would do.
pub fn print_analysis(plan: &SyncPlan, settings: &Settings) {
    let s = &plan.summary;
    println!(
        "Scanned {} source / {} destination files",
        format_count(s.source_files),
        format_count(s.destination_files)
    );
    println!();
    row(style("+").green(), "Added", &format_count(s.added.planned), sized(s.bytes_added));
    row(style("~").yellow(), "Modified", &format_count(s.modified.planned), sized(s.bytes_modified));
    row(style("-").red(), "Removed", &format_count(s.removed.planned), String::new());
    print_optional_rows(plan, |c| format_count(c.planned));
    row(style("=").dim(), "Unchanged", &format_count(s.unchanged), String::new());

    print_issues(plan, &plan.warnings, settings);
}

/// Outcome of the sync: done / planned per category, failures and problems.
pub fn print_sync_result(plan: &SyncPlan, settings: &Settings) {
    let s = &plan.summary;
    let Some(execution) = &plan.execution else { return };
    println!();
    let headline = format!("Sync finished in {}", duration(execution.duration_secs));
    if execution.cancelled {
        println!("{} (cancelled before the end)", style(headline).yellow().bold());
    } else {
        println!("{}", style(headline).bold());
    }
    row(style("+").green(), "Added", &progress_of(&s.added), String::new());
    row(style("~").yellow(), "Modified", &progress_of(&s.modified), String::new());
    let removed_note = match &execution.trash_folder {
        Some(folder) => format!("(moved to {folder})"),
        None => format!("({})", execution.delete_mode_used),
    };
    let removed_note = if s.removed.done > 0 { removed_note } else { String::new() };
    row(style("-").red(), "Removed", &progress_of(&s.removed), removed_note);
    print_optional_rows(plan, progress_of);
    row(style("=").dim(), "Unchanged", &format_count(s.unchanged), String::new());

    print_issues(plan, &execution.warnings, settings);
}

pub fn print_in_sync() {
    println!();
    println!("{}", style("✔ Already in sync, nothing to do.").green().bold());
}

pub fn print_declined() {
    println!("Sync cancelled. Nothing was changed.");
}

pub fn print_not_interactive() {
    println!();
    println!(
        "{} Not running in an interactive terminal, so no confirmation can be asked. Add --yes to sync unattended.",
        style("✖").red()
    );
}

pub fn print_footer(report: &Path, html: &Path, code: i32) {
    let full = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).display().to_string();
    println!();
    println!("Result file: {}", full(report));
    println!("HTML report: {}", full(html));
    println!("Exit code: {code} ({})", exit_code_meaning(code));
}

pub fn print_fatal(err: &anyhow::Error) {
    eprintln!("{} {err:#}", style("error:").red().bold());
}

fn print_optional_rows(plan: &SyncPlan, value: impl Fn(&Counter) -> String) {
    let s = &plan.summary;
    if s.time_updated.planned > 0 {
        row(style("↻").cyan(), "Time only", &value(&s.time_updated), "(content identical, timestamp updated)".into());
    }
    if s.renamed.planned > 0 {
        row(style("⇄").cyan(), "Renamed", &value(&s.renamed), "(letter case only)".into());
    }
    if s.dirs_created.planned > 0 {
        row(style("▸").green(), "New folders", &value(&s.dirs_created), String::new());
    }
    if s.dirs_removed.planned > 0 {
        row(style("▸").red(), "Old folders", &value(&s.dirs_removed), "(removed)".into());
    }
}

fn print_issues(plan: &SyncPlan, warnings: &[String], settings: &Settings) {
    if !warnings.is_empty() {
        println!();
        for warning in warnings {
            println!("{} {warning}", style("⚠").yellow());
        }
    }
    if !plan.skipped.is_empty() {
        println!();
        println!(
            "{} {} entr{} skipped (symbolic links, special or online-only files); details in the result file",
            style("ℹ").cyan(),
            format_count(plan.skipped.len() as u64),
            if plan.skipped.len() == 1 { "y" } else { "ies" }
        );
    }
    print_problems(&plan.problems, settings.max_errors_shown);
    if !plan.blockers.is_empty() {
        println!();
        println!("{}", style("✖ Sync blocked by safety checks:").red().bold());
        for blocker in &plan.blockers {
            println!("  - {blocker}");
        }
    }
}

/// Up to `max_shown` problems: list each path. More: counts by reason.
fn print_problems(problems: &[Problem], max_shown: usize) {
    if problems.is_empty() {
        return;
    }
    println!();
    println!(
        "{} {} problem{}:",
        style("⚠").yellow(),
        format_count(problems.len() as u64),
        if problems.len() == 1 { "" } else { "s" }
    );
    if problems.len() <= max_shown {
        let width = problems.iter().map(|p| p.reason.as_str().len()).max().unwrap_or(0) + 2;
        for problem in problems {
            let label = format!("[{}]", problem.reason);
            println!("  {}  {}", style(format!("{label:<width$}")).yellow(), problem.path());
        }
    } else {
        let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
        for problem in problems {
            *counts.entry(problem.reason.as_str()).or_default() += 1;
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (reason, count) in counts {
            println!("  {:<26} {:>8}", style(reason).yellow(), format_count(count));
        }
        println!("  Full list with paths in the result file.");
    }
}

fn row(symbol: StyledObject<&str>, label: &str, value: &str, note: String) {
    println!("  {symbol} {:<12} {value:>10}  {}", format!("{label}:"), style(note).dim());
}

fn sized(bytes: u64) -> String {
    if bytes > 0 {
        format!("({})", format_bytes(bytes))
    } else {
        String::new()
    }
}

fn progress_of(counter: &Counter) -> String {
    let mut text = format!("{} / {}", format_count(counter.done), format_count(counter.planned));
    if counter.failed > 0 {
        text.push_str(&format!(" ({} failed)", format_count(counter.failed)));
    }
    if counter.skipped > 0 {
        text.push_str(&format!(" ({} skipped)", format_count(counter.skipped)));
    }
    text
}

fn duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    match total {
        0..=59 => format!("{seconds:.1}s"),
        60..=3599 => format!("{}m {}s", total / 60, total % 60),
        _ => format!("{}h {}m", total / 3600, (total % 3600) / 60),
    }
}
