//! End-to-end scenarios: real temporary folders, full analysis and sync.

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;

use common::*;
use folder_mirror::app;
use folder_mirror::features::plan::{
    ActionKind, ActionReason, ActionStatus, SyncPlan, EXIT_ABORTED, EXIT_DIFFERENCES, EXIT_OK, EXIT_PROBLEMS,
};
use folder_mirror::features::report::{self, ReportFormat};
use folder_mirror::shared::config::{CompareMode, DeleteMode};
use folder_mirror::shared::error::{MirrorError, ProblemReason};
use folder_mirror::shared::paths::detect_case_insensitive;
use folder_mirror::shared::progress::NoProgress;

fn kinds(plan: &SyncPlan) -> Vec<(ActionKind, String)> {
    plan.actions.iter().map(|a| (a.action, a.path.to_string())).collect()
}

fn action<'a>(plan: &'a SyncPlan, path: &str) -> &'a folder_mirror::features::plan::SyncAction {
    plan.actions.iter().find(|a| a.path.as_str() == path).unwrap_or_else(|| panic!("no action for {path}"))
}

// ---------- Basic mirroring ----------

#[test]
fn copies_new_files_and_folders() {
    let f = Fixture::new();
    f.src("a.txt", "a").src("docs/deep/b.txt", "b");
    fs::create_dir_all(f.source.join("empty")).unwrap();

    let plan = f.mirror();

    assert_eq!(plan.summary.added.done, 2);
    assert_eq!(tree(&f.destination), tree(&f.source));
    assert_eq!(folders(&f.destination), folders(&f.source));
    assert_eq!(plan.exit_code(), EXIT_OK);
}

#[test]
fn second_run_has_nothing_to_do() {
    let f = Fixture::new();
    f.src("a.txt", "a").dst("old.txt", "old");
    f.mirror();

    let plan = f.analyze(&f.settings());

    assert!(plan.actions.is_empty(), "unexpected actions: {:?}", kinds(&plan));
    assert_eq!(plan.exit_code(), EXIT_OK);
}

#[test]
fn analysis_alone_changes_nothing() {
    let f = Fixture::new();
    f.src("new.txt", "new").dst("old.txt", "old");

    let plan = f.analyze(&f.settings());

    assert_eq!(plan.exit_code(), EXIT_DIFFERENCES);
    assert_eq!(tree(&f.destination).keys().collect::<Vec<_>>(), ["old.txt"]);
}

#[test]
fn creates_a_missing_destination() {
    let f = Fixture::new();
    fs::remove_dir(&f.destination).unwrap();
    f.src("a.txt", "a");

    let plan = f.mirror();

    assert!(plan.warnings.iter().any(|w| w.contains("does not exist")));
    assert_eq!(tree(&f.destination), tree(&f.source));
}

// ---------- Modified files ----------

#[test]
fn overwrites_files_whose_size_changed() {
    let f = Fixture::new();
    f.src("data.txt", "new and longer").dst("data.txt", "old");

    let plan = f.mirror();

    let data = action(&plan, "data.txt");
    assert_eq!((data.action, data.reason), (ActionKind::Overwrite, ActionReason::SizeChanged));
    assert_eq!(tree(&f.destination)["data.txt"], "new and longer");
}

#[test]
fn smart_mode_detects_same_size_edit_with_new_time() {
    let f = Fixture::new();
    f.src_at("config.ini", "port=9090", NEW).dst_at("config.ini", "port=8080", OLD);

    let plan = f.mirror();

    assert_eq!(action(&plan, "config.ini").reason, ActionReason::ContentChanged);
    assert_eq!(tree(&f.destination)["config.ini"], "port=9090");
}

#[test]
fn only_updates_the_time_when_content_is_identical() {
    let f = Fixture::new();
    f.src_at("same.txt", "same", NEW).dst_at("same.txt", "same", OLD);

    let plan = f.mirror();

    assert_eq!(action(&plan, "same.txt").action, ActionKind::UpdateTime);
    assert_eq!(modified(&f.destination.join("same.txt")), at(NEW));
    assert!(f.trash().is_empty(), "nothing should be backed up for a time-only change");
}

#[test]
fn copies_keep_the_source_modification_time() {
    let f = Fixture::new();
    f.src_at("a.txt", "a", NEW);

    f.mirror();

    assert_eq!(modified(&f.destination.join("a.txt")), at(NEW));
}

#[test]
fn times_within_the_tolerance_count_as_equal() {
    let f = Fixture::new();
    f.src_at("a.txt", "aaa", OLD + 1).dst_at("a.txt", "bbb", OLD);

    let plan = f.analyze(&f.settings());

    assert!(plan.actions.is_empty(), "1 s difference is within the default 2 s tolerance");
}

#[test]
fn compare_modes_detect_different_kinds_of_change() {
    // Same size and same time, different content: only a full hash notices.
    let f = Fixture::new();
    f.src("x.txt", "abc").dst("x.txt", "abd");
    let mut settings = f.settings();

    for (mode, detected) in [
        (CompareMode::Name, false),
        (CompareMode::Metadata, false),
        (CompareMode::Smart, false),
        (CompareMode::QuickHash, true), // small files are hashed completely
        (CompareMode::FullHash, true),
    ] {
        settings.mode = mode;
        let plan = f.analyze(&settings);
        assert_eq!(!plan.actions.is_empty(), detected, "mode {mode}");
    }
}

#[test]
fn name_mode_never_reports_edits() {
    let f = Fixture::new();
    f.src_at("a.txt", "a much longer content", NEW).dst("a.txt", "short");
    let mut settings = f.settings();
    settings.mode = CompareMode::Name;

    let plan = f.analyze(&settings);

    assert!(plan.actions.is_empty());
}

#[test]
fn overwrites_read_only_destination_files() {
    let f = Fixture::new();
    f.src("ro.txt", "new content").dst("ro.txt", "old");
    let path = f.destination.join("ro.txt");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();

    let plan = f.mirror();

    assert_eq!(plan.exit_code(), EXIT_OK, "problems: {:?}", plan.problems);
    assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
}

// ---------- Deletions ----------

#[test]
fn deletes_files_and_folders_only_in_the_destination() {
    let f = Fixture::new();
    f.both("keep.txt", "keep").dst("old.txt", "old").dst("olddir/x.txt", "x");

    let plan = f.mirror();

    assert_eq!(plan.summary.removed.done, 2);
    assert_eq!(plan.summary.dirs_removed.done, 1);
    assert_eq!(tree(&f.destination).keys().collect::<Vec<_>>(), ["keep.txt"]);
    assert!(folders(&f.destination).is_empty());
    assert_eq!(f.trash(), ["old.txt", "olddir/x.txt"].map(String::from).into());
}

#[test]
fn keeps_the_old_version_of_overwritten_files() {
    let f = Fixture::new();
    f.src("doc.txt", "version 2").dst("doc.txt", "v1");

    f.mirror();

    assert_eq!(f.trash(), ["doc.txt".to_string()].into());
    assert_eq!(tree(&f.destination)["doc.txt"], "version 2");
}

#[test]
fn no_backup_when_disabled() {
    let f = Fixture::new();
    f.src("doc.txt", "version 2").dst("doc.txt", "v1");
    let mut settings = f.settings();
    settings.backup_overwritten = false;

    f.mirror_with(&settings);

    assert!(f.trash().is_empty());
}

#[test]
fn permanent_mode_leaves_no_trash() {
    let f = Fixture::new();
    f.dst("old.txt", "old");
    let mut settings = f.settings();
    settings.delete_mode = DeleteMode::Permanent;

    let plan = f.mirror_with(&settings);

    assert_eq!(plan.summary.removed.done, 1);
    assert!(!f.destination.join(".mirror-trash").exists());
    assert!(tree(&f.destination).is_empty());
}

#[test]
fn removes_folders_that_only_contain_excluded_files() {
    let f = Fixture::new();
    f.dst("olddir/Thumbs.db", "thumbs");

    let plan = f.mirror();

    assert_eq!(plan.summary.dirs_removed.done, 1);
    assert!(!f.destination.join("olddir").exists());
}

#[test]
fn trash_keeps_only_the_latest_runs() {
    let f = Fixture::new();
    for run in ["2020-01-01_000000", "2020-02-01_000000", "2020-03-01_000000"] {
        fs::create_dir_all(f.destination.join(".mirror-trash").join(run)).unwrap();
    }
    f.dst("old.txt", "old");
    let mut settings = f.settings();
    settings.trash_keep = 2;

    f.mirror_with(&settings);

    let runs: Vec<_> = fs::read_dir(f.destination.join(".mirror-trash")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(runs.len(), 2);
    assert!(!runs.iter().any(|r| r == "2020-01-01_000000" || r == "2020-02-01_000000"));
}

// ---------- Exclusions and special names ----------

#[test]
fn excluded_files_are_never_copied_or_deleted() {
    let f = Fixture::new();
    f.src("a.txt", "a").src("notes.tmp", "tmp").src("Thumbs.db", "source thumbs");
    f.dst("Thumbs.db", "destination thumbs").dst("cache.tmp", "keep me");
    let mut settings = f.settings();
    settings.excludes = vec!["*.tmp".into()];

    f.mirror_with(&settings);

    let result = tree(&f.destination);
    assert_eq!(result.keys().collect::<Vec<_>>(), ["Thumbs.db", "a.txt", "cache.tmp"]);
    assert_eq!(result["Thumbs.db"], "destination thumbs");
}

#[test]
fn renames_when_only_the_letter_case_differs() {
    let f = Fixture::new();
    if !detect_case_insensitive(&f.destination) {
        return; // only meaningful on case-insensitive file systems (Windows, macOS)
    }
    f.src("Report.txt", "same").dst("report.txt", "same");

    let plan = f.mirror();

    assert_eq!(kinds(&plan), [(ActionKind::RenameCase, "Report.txt".to_string())]);
    let names: Vec<_> = fs::read_dir(&f.destination).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, ["Report.txt"]);
}

// ---------- Safety ----------

#[test]
fn blocks_a_sync_that_would_delete_too_much() {
    let f = Fixture::new();
    for i in 0..20 {
        f.dst(&format!("f{i}.txt"), "x");
    }
    for i in 0..5 {
        f.src(&format!("f{i}.txt"), "x");
    }
    let settings = f.settings();
    let mut plan = f.analyze(&settings);

    assert_eq!(plan.blockers.len(), 1, "15 of 20 files is above the 20% limit");
    assert_eq!(plan.exit_code(), EXIT_ABORTED);
    let result = app::sync(&mut plan, &settings, &NoProgress, &AtomicBool::new(false), "test");
    assert!(matches!(result, Err(MirrorError::Blocked(_))));
    assert_eq!(tree(&f.destination).len(), 20, "nothing was deleted");
}

#[test]
fn a_higher_limit_allows_large_deletions() {
    let f = Fixture::new();
    for i in 0..20 {
        f.dst(&format!("f{i}.txt"), "x");
    }
    let mut settings = f.settings();
    settings.max_delete_percent = 100.0;

    let plan = f.mirror_with(&settings);

    assert_eq!(plan.summary.removed.done, 20);
}

#[test]
fn few_deletions_are_never_blocked() {
    let f = Fixture::new();
    f.both("keep.txt", "k").dst("a.txt", "a").dst("b.txt", "b");

    let plan = f.analyze(&f.settings());

    assert!(plan.blockers.is_empty(), "2 of 3 files is above 20%, but below the minimum of 10");
}

#[test]
fn rejects_the_same_folder() {
    let f = Fixture::new();
    let mut settings = f.settings();
    settings.destination = f.source.clone();

    let error = app::analyze(&settings, &NoProgress).unwrap_err();

    assert!(matches!(error, MirrorError::Overlap(..)), "{error}");
}

#[test]
fn rejects_a_destination_inside_the_source() {
    let f = Fixture::new();
    let mut settings = f.settings();
    settings.destination = f.source.join("mirror");

    assert!(matches!(app::analyze(&settings, &NoProgress), Err(MirrorError::Overlap(..))));
}

#[test]
fn rejects_a_source_inside_the_destination() {
    let f = Fixture::new();
    let mut settings = f.settings();
    settings.source = f.destination.join("work");
    fs::create_dir_all(&settings.source).unwrap();

    assert!(matches!(app::analyze(&settings, &NoProgress), Err(MirrorError::Overlap(..))));
}

#[test]
fn a_missing_source_is_an_error() {
    let f = Fixture::new();
    let mut settings = f.settings();
    settings.source = f.root().join("does-not-exist");

    assert!(matches!(app::analyze(&settings, &NoProgress), Err(MirrorError::SourceNotFound(_))));
}

#[test]
fn skips_files_that_changed_after_the_analysis() {
    let f = Fixture::new();
    f.src("a.txt", "first");
    let settings = f.settings();
    let mut plan = f.analyze(&settings);
    f.src_at("a.txt", "changed later", NEW);

    f.sync(&mut plan, &settings);

    let a = action(&plan, "a.txt");
    assert_eq!(a.status, ActionStatus::Skipped);
    assert_eq!(a.problem, Some(ProblemReason::SourceChangedSincePlan));
    assert_eq!(plan.exit_code(), EXIT_PROBLEMS);
    assert!(!f.destination.join("a.txt").exists());
}

#[test]
fn does_not_delete_a_file_that_came_back_to_the_source() {
    let f = Fixture::new();
    f.dst("old.txt", "old");
    let settings = f.settings();
    let mut plan = f.analyze(&settings);
    f.src("old.txt", "it is back");

    f.sync(&mut plan, &settings);

    let old = action(&plan, "old.txt");
    assert_eq!((old.status, old.problem), (ActionStatus::Skipped, Some(ProblemReason::SourceChangedSincePlan)));
    assert!(f.destination.join("old.txt").exists());
}

#[test]
fn protects_destination_content_behind_a_skipped_source_link() {
    let f = Fixture::new();
    let target = f.root().join("outside");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("t.txt"), "t").unwrap();
    if !make_dir_link(&target, &f.source.join("linked")) {
        eprintln!("skipped: cannot create a directory link here");
        return;
    }
    f.src("a.txt", "a").dst("linked/inside.txt", "keep me");

    let plan = f.mirror();

    assert_eq!(plan.skipped.len(), 1, "the link is reported as skipped");
    assert_eq!(plan.skipped[0].reason, ProblemReason::Symlink);
    assert!(f.destination.join("linked/inside.txt").exists(), "content behind the link is protected");
    assert_eq!(plan.summary.protected, 1);
    assert!(plan.actions.iter().all(|a| !a.path.as_str().starts_with("linked")));
}

/// Creates a directory link: a junction on Windows (no admin rights needed), a symlink elsewhere.
fn make_dir_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

#[test]
fn renames_folders_when_only_the_letter_case_differs() {
    let f = Fixture::new();
    if !detect_case_insensitive(&f.destination) {
        return;
    }
    f.src("Docs/a.txt", "a").dst("docs/a.txt", "a");

    let plan = f.mirror();

    assert_eq!(kinds(&plan), [(ActionKind::RenameCase, "Docs".to_string())]);
    let names: Vec<_> = fs::read_dir(&f.destination).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, ["Docs"]);
}

// ---------- Cancellation and errors ----------

#[test]
fn a_cancelled_sync_changes_nothing_more() {
    let f = Fixture::new();
    f.src("a.txt", "a").src("b.txt", "b");
    let settings = f.settings();
    let mut plan = f.analyze(&settings);

    app::sync(&mut plan, &settings, &NoProgress, &AtomicBool::new(true), "test").unwrap();

    assert!(plan.actions.iter().all(|a| a.problem == Some(ProblemReason::Cancelled)));
    assert!(plan.execution.as_ref().unwrap().cancelled);
    assert_eq!(plan.exit_code(), EXIT_PROBLEMS);
    assert!(tree(&f.destination).is_empty());
    assert!(plan.problems.is_empty(), "cancelled actions are not reported as problems");
}

#[test]
fn a_failed_file_does_not_stop_the_sync_by_default() {
    let f = Fixture::new();
    f.src("a.txt", "a").src("b.txt", "b").src("c.txt", "c");
    let settings = f.settings();
    let mut plan = f.analyze(&settings);
    fs::create_dir(f.destination.join("a.txt")).unwrap(); // a folder now blocks the copy of a.txt

    f.sync(&mut plan, &settings);

    assert_eq!(action(&plan, "a.txt").status, ActionStatus::Failed);
    assert_eq!(plan.summary.added.done, 2);
    assert_eq!(plan.problems.len(), 1);
    assert_eq!(plan.exit_code(), EXIT_PROBLEMS);
}

#[test]
fn stop_on_error_skips_the_rest_after_a_failure() {
    let f = Fixture::new();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        f.src(name, name);
    }
    let mut settings = f.settings();
    settings.stop_on_error = true;
    settings.threads = Some(1);
    let mut plan = f.analyze(&settings);
    fs::create_dir(f.destination.join("a.txt")).unwrap();

    // One worker so the order is predictable: a.txt fails first, the rest are cancelled.
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    pool.install(|| f.sync(&mut plan, &settings));

    assert_eq!(action(&plan, "a.txt").status, ActionStatus::Failed);
    assert_eq!(plan.summary.added.done, 0);
    assert!(plan.execution.as_ref().unwrap().cancelled);
}

// ---------- Plans and reports ----------

#[test]
fn a_saved_plan_can_be_applied_later() {
    let f = Fixture::new();
    f.src("a.txt", "a").dst("old.txt", "old");
    let settings = f.settings();
    let plan = f.analyze(&settings);
    let file = f.root().join("plan.json");
    report::write_report(&plan, &file, ReportFormat::Json).unwrap();

    let mut loaded = app::load_plan(&file).unwrap();
    f.sync(&mut loaded, &settings);

    assert_eq!(tree(&f.destination), tree(&f.source));
    assert_eq!(loaded.exit_code(), EXIT_OK);
}

#[test]
fn report_names_are_unique_and_paired() {
    let f = Fixture::new();
    let dir = f.root().join("reports");
    let time = chrono::Local::now();

    let names: Vec<_> = (0..3)
        .map(|_| report::report_paths(&dir, ReportFormat::Json, time).unwrap())
        .map(|paths| {
            assert_eq!(paths.result.with_extension("html"), paths.html);
            paths.result.file_name().unwrap().to_string_lossy().into_owned()
        })
        .collect();

    assert!(names[0].starts_with("folder-mirror-") && names[0].ends_with(".json"));
    assert!(names[1].ends_with("-2.json") && names[2].ends_with("-3.json"));
}

#[test]
fn reports_are_written_in_every_format() {
    let f = Fixture::new();
    f.src("a.txt", "a");
    let mut plan = f.mirror();
    plan.not_executed = None;

    let json = f.root().join("r.json");
    let csv = f.root().join("r.csv");
    let html = f.root().join("r.html");
    report::write_report(&plan, &json, ReportFormat::Json).unwrap();
    report::write_report(&plan, &csv, ReportFormat::Csv).unwrap();
    report::write_html(&plan, &html, plan.exit_code()).unwrap();

    assert!(report::read_plan(&json).unwrap().execution.is_some());
    assert!(fs::read_to_string(&csv).unwrap().starts_with("action,reason,status"));
    let page = fs::read_to_string(&html).unwrap();
    assert!(!page.contains("__REPORT_DATA__"));
    assert!(page.contains("\"exit_code\":0"));
}
