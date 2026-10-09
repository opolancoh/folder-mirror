//! Helpers shared by the integration tests: temporary source/destination folders and shortcuts
//! to run the analysis and the sync.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use folder_mirror::app;
use folder_mirror::features::plan::SyncPlan;
use folder_mirror::shared::config::{DeleteMode, Settings};
use folder_mirror::shared::files::TRASH_DIR_NAME;
use folder_mirror::shared::progress::NoProgress;
use tempfile::TempDir;
use walkdir::WalkDir;

/// Default modification time of test files (Sept 2020).
pub const OLD: u64 = 1_600_000_000;
/// A later modification time (Nov 2023).
pub const NEW: u64 = 1_700_000_000;

pub fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// A temporary `source` and `destination` folder, deleted at the end of the test.
pub struct Fixture {
    dir: TempDir,
    pub source: PathBuf,
    pub destination: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("cannot create temp dir");
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        Self { dir, source, destination }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Writes a source file with the default (old) time.
    pub fn src(&self, rel: &str, content: &str) -> &Self {
        write(&self.source, rel, content, OLD);
        self
    }

    /// Writes a destination file with the default (old) time.
    pub fn dst(&self, rel: &str, content: &str) -> &Self {
        write(&self.destination, rel, content, OLD);
        self
    }

    /// Writes the same file on both sides (already in sync).
    pub fn both(&self, rel: &str, content: &str) -> &Self {
        self.src(rel, content).dst(rel, content)
    }

    pub fn src_at(&self, rel: &str, content: &str, seconds: u64) -> &Self {
        write(&self.source, rel, content, seconds);
        self
    }

    pub fn dst_at(&self, rel: &str, content: &str, seconds: u64) -> &Self {
        write(&self.destination, rel, content, seconds);
        self
    }

    /// Test settings: deletions go to the trash folder (never the real Recycle Bin), short retries.
    pub fn settings(&self) -> Settings {
        let mut settings = Settings::new(&self.source, &self.destination);
        settings.delete_mode = DeleteMode::TrashFolder;
        settings.retry_delay = Duration::from_millis(10);
        settings
    }

    pub fn analyze(&self, settings: &Settings) -> SyncPlan {
        app::analyze(settings, &NoProgress).expect("analysis failed")
    }

    pub fn sync(&self, plan: &mut SyncPlan, settings: &Settings) {
        app::sync(plan, settings, &NoProgress, &AtomicBool::new(false), "test").expect("sync failed");
    }

    /// Analyze and sync with `settings`; returns the executed plan.
    pub fn mirror_with(&self, settings: &Settings) -> SyncPlan {
        let mut plan = self.analyze(settings);
        self.sync(&mut plan, settings);
        plan
    }

    /// Analyze and sync with the default test settings.
    pub fn mirror(&self) -> SyncPlan {
        self.mirror_with(&self.settings())
    }

    /// Files under `.mirror-trash` (relative to the trash run folder).
    pub fn trash(&self) -> BTreeSet<String> {
        let trash = self.destination.join(TRASH_DIR_NAME);
        WalkDir::new(&trash)
            .min_depth(2)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                let rel = e.path().strip_prefix(&trash).unwrap();
                // Drop the run folder (first component).
                rel.components().skip(1).map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
            })
            .collect()
    }
}

pub fn native(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |path, part| path.join(part))
}

pub fn write(root: &Path, rel: &str, content: &str, seconds: u64) {
    let path = native(root, rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    set_time(&path, seconds);
}

pub fn set_time(path: &Path, seconds: u64) {
    File::options().write(true).open(path).unwrap().set_modified(at(seconds)).unwrap();
}

pub fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// Every file under `root` as relative path → content, ignoring the trash folder.
pub fn tree(root: &Path) -> BTreeMap<String, String> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != TRASH_DIR_NAME)
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            (rel, fs::read_to_string(e.path()).unwrap_or_default())
        })
        .collect()
}

/// Every folder under `root` (relative paths), ignoring the trash folder.
pub fn folders(root: &Path) -> BTreeSet<String> {
    WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| e.file_name() != TRASH_DIR_NAME)
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
        .collect()
}
