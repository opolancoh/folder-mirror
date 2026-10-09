//! Console output: summaries, warnings and problems.

use std::collections::BTreeMap;
use std::path::Path;

use console::{style, StyledObject};

use folder_mirror::features::plan::{exit_code_meaning, Counter, SyncPlan};
use folder_mirror::shared::config::{CompareMode, DeleteMode, Settings};
use folder_mirror::shared::error::Problem;
use folder_mirror::shared::files::TRASH_DIR_NAME;
use folder_mirror::shared::format::{format_bytes, format_count};

/// Shown when the tool runs without a command.
pub fn print_overview() {
    println!("{}", style(concat!("mirror ", env!("CARGO_PKG_VERSION"), " · keeps a mirror folder identical to its source folder (one-way)")).bold());
    println!();
    println!("{}", style("How it works").bold());
    println!("  1. Scans both folders and shows what will change in the destination.");
    println!("  2. Asks \"Proceed with sync?\" (skip the question with --confirm false).");
    println!("  3. Copies new files, overwrites edited files and deletes files that are no longer");
    println!("     in the source, so the destination ends up identical to the source.");
    println!("  The source is never changed. Each run writes a result file and an HTML report with");
    println!("  every file's full path and action. Failed files never stop the sync.");
    println!();
    println!("{}", style("Default settings").bold());
    line("Confirmation", "asks before changing anything (--confirm true)".into());
    print_setting_lines(&Settings::new("", ""));
    println!();
    println!("{}", style("Examples").bold());
    println!("  mirror -s \"C:\\source\" -d \"D:\\mirror\"                   analyze, ask, apply");
    println!("  mirror -s \"C:\\source\" -d \"D:\\mirror\" --dry-run         only show the differences");
    println!("  mirror -s \"C:\\source\" -d \"D:\\mirror\" --confirm false   apply without asking");
    println!("  mirror --help                                           all options");
}

/// Whether the run asks, applies automatically, or only reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Ask,
    Automatic,
    DryRun,
}

/// How this run will work, from the effective settings (defaults or given by the user).
pub fn print_settings(settings: &Settings, mode: RunMode) {
    println!();
    let what = match mode {
        RunMode::Ask => "makes the destination identical to the source, after your confirmation",
        RunMode::Automatic => "makes the destination identical to the source, without asking (--confirm false)",
        RunMode::DryRun => "dry run: shows the differences and writes the report; nothing is changed",
    };
    println!("{} · {what}", style(concat!("mirror ", env!("CARGO_PKG_VERSION"))).bold());
    let confirmation = match mode {
        RunMode::Ask => "asks before changing anything",
        RunMode::Automatic => "no, changes are applied automatically",
        RunMode::DryRun => "not needed, nothing will be changed",
    };
    line("Confirmation", confirmation.into());
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

/// What the sync will change in the destination.
pub fn print_analysis(plan: &SyncPlan, settings: &Settings) {
    let s = &plan.summary;
    print_sides(plan);
    println!();
    if plan.actions.is_empty() {
        println!(
            "Changes to the destination ({}): {}, it is already identical to the source ({}).",
            plan.destination,
            style("none").green().bold(),
            files(s.source_files)
        );
    } else {
        println!("{}", style(format!("Changes to the destination ({}):", plan.destination)).bold());
        print_change_rows(plan, |c| format_count(c.planned));
        let after = s.destination_files + s.added.planned - s.removed.planned;
        let same = if after == s.source_files { ", the same as the source" } else { "" };
        println!();
        println!("After sync: the destination will have {}{same}.", files(after));
    }
    print_issues(plan, &plan.warnings, settings);
}

/// What the sync changed in the destination: done / planned per line.
pub fn print_sync_result(plan: &SyncPlan, settings: &Settings) {
    let Some(execution) = &plan.execution else { return };
    println!();
    let headline = format!(
        "Sync finished in {} · changes made to the destination ({}):",
        duration(execution.duration_secs),
        plan.destination
    );
    println!("{}", style(headline).bold());
    print_change_rows(plan, |c| format!("{} / {}", format_count(c.done), format_count(c.planned)));
    println!();
    if execution.cancelled {
        println!("{}", style("The sync was cancelled before the end; run it again to finish.").yellow());
    } else if plan.summary.problems.total > 0 {
        println!("{}", style("Some changes could not be made; see the problems below and run the sync again.").yellow());
    } else {
        println!("{}", style("The destination is now identical to the source.").green());
    }
    print_issues(plan, &execution.warnings, settings);
}

pub fn print_declined() {
    println!("Sync cancelled. Nothing was changed.");
}

pub fn print_not_interactive() {
    println!();
    println!(
        "{} Not running in an interactive terminal, so no confirmation can be asked. Nothing was changed; add --confirm false to apply changes unattended.",
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

fn print_sides(plan: &SyncPlan) {
    let s = &plan.summary;
    let width = plan.source.chars().count().max(plan.destination.chars().count());
    println!("{:<12} {:<width$}   {}", "Source", plan.source, files(s.source_files));
    println!("{:<12} {:<width$}   {}", "Destination", plan.destination, files(s.destination_files));
}

/// One line per kind of change; `value` renders the count (planned, or done / planned).
fn print_change_rows(plan: &SyncPlan, value: impl Fn(&Counter) -> String) {
    struct Row {
        symbol: StyledObject<&'static str>,
        label: &'static str,
        count: String,
        unit: &'static str,
        size: String,
        note: String,
        failures: String,
    }

    let s = &plan.summary;
    let mut rows = Vec::new();
    let mut add = |symbol, label, counter: &Counter, unit: &'static str, bytes: u64, note: String| {
        let unit = if unit == "files" && counter.planned == 1 { "file" } else { unit };
        let mut failures = Vec::new();
        if counter.failed > 0 {
            failures.push(format!("{} failed", format_count(counter.failed)));
        }
        if counter.skipped > 0 {
            failures.push(format!("{} skipped", format_count(counter.skipped)));
        }
        rows.push(Row {
            symbol,
            label,
            count: value(counter),
            unit,
            size: if bytes > 0 { format_bytes(bytes) } else { String::new() },
            note,
            failures: if failures.is_empty() { String::new() } else { format!("  ({})", failures.join(", ")) },
        });
    };

    add(style("+").green(), "Copy (new)", &s.added, "files", s.bytes_added, "in the source, missing in the destination".into());
    add(style("~").yellow(), "Overwrite", &s.modified, "files", s.bytes_modified, "changed in the source".into());
    add(style("-").red(), "Delete", &s.removed, "files", s.bytes_removed, format!("no longer in the source {}", delete_target(plan)));
    if s.renamed.planned > 0 {
        add(style("⇄").cyan(), "Rename", &s.renamed, "files", 0, "letter case only".into());
    }
    if s.time_updated.planned > 0 {
        add(style("↻").cyan(), "Update time", &s.time_updated, "files", 0, "same content, timestamp only".into());
    }
    if s.dirs_created.planned > 0 {
        add(style("▸").green(), "Create folders", &s.dirs_created, "", 0, String::new());
    }
    if s.dirs_removed.planned > 0 {
        add(style("▸").red(), "Delete folders", &s.dirs_removed, "", 0, "no longer in the source".into());
    }
    rows.push(Row {
        symbol: style("=").dim(),
        label: "Already identical",
        count: format_count(s.unchanged),
        unit: if s.unchanged == 1 { "file" } else { "files" },
        size: String::new(),
        note: String::new(),
        failures: String::new(),
    });

    let count_width = rows.iter().map(|r| r.count.len()).max().unwrap_or(0);
    let size_width = rows.iter().map(|r| r.size.len()).max().unwrap_or(0);
    for row in rows {
        println!(
            "  {} {:<17} {:>count_width$} {:<5}  {:>size_width$}   {}{}",
            row.symbol,
            row.label,
            row.count,
            row.unit,
            row.size,
            style(row.note).dim(),
            style(row.failures).red()
        );
    }
}

/// Where deleted files go, for the summary.
fn delete_target(plan: &SyncPlan) -> String {
    if let Some(execution) = &plan.execution {
        if let Some(folder) = &execution.trash_folder {
            // The run folder inside the destination: .mirror-trash\<date>
            let run = Path::new(folder).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return format!("→ {}", Path::new(TRASH_DIR_NAME).join(run).display());
        }
        return delete_mode_target(execution.delete_mode_used).to_string();
    }
    delete_mode_target(plan.delete_mode).to_string()
}

fn delete_mode_target(mode: DeleteMode) -> &'static str {
    match mode {
        DeleteMode::RecycleBin => "→ Recycle Bin",
        DeleteMode::TrashFolder => "→ .mirror-trash folder",
        DeleteMode::Permanent => "→ deleted permanently",
    }
}

fn files(count: u64) -> String {
    format!("{} file{}", format_count(count), if count == 1 { "" } else { "s" })
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

fn duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    match total {
        0..=59 => format!("{seconds:.1}s"),
        60..=3599 => format!("{}m {}s", total / 60, total % 60),
        _ => format!("{}h {}m", total / 3600, (total % 3600) / 60),
    }
}
