//! Removes destination entries according to the delete mode, with automatic fallback to the
//! trash folder when the Recycle Bin is not available.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::shared::config::DeleteMode;
use crate::shared::files::{self, TRASH_DIR_NAME};
use crate::shared::paths::{display_path, plain_path, RelPath};

pub struct Deleter {
    mode: DeleteMode,
    trash_dir: PathBuf,
    fallback: Mutex<Option<String>>,
    trash_used: AtomicBool,
    /// Serializes trash moves so parallel workers never pick the same target name.
    trash_lock: Mutex<()>,
}

impl Deleter {
    pub fn new(requested: DeleteMode, destination_root: &Path, run_id: &str) -> Self {
        let trash_dir = destination_root.join(TRASH_DIR_NAME).join(run_id);
        let mut fallback = None;
        let mode = match requested {
            DeleteMode::RecycleBin => match files::recycle_bin_unavailable_reason(destination_root) {
                Some(why) => {
                    fallback = Some(format!(
                        "Recycle Bin not available for {} ({why}); used {} instead",
                        display_path(destination_root),
                        display_path(&trash_dir)
                    ));
                    DeleteMode::TrashFolder
                }
                None => DeleteMode::RecycleBin,
            },
            other => other,
        };
        Self {
            mode,
            trash_dir,
            fallback: Mutex::new(fallback),
            trash_used: AtomicBool::new(false),
            trash_lock: Mutex::new(()),
        }
    }

    pub fn effective_mode(&self) -> DeleteMode {
        self.mode
    }

    /// Explains why (part of) the deletions did not go to the Recycle Bin.
    pub fn fallback_note(&self) -> Option<String> {
        self.fallback.lock().unwrap().clone()
    }

    pub fn trash_dir_used(&self) -> Option<&Path> {
        self.trash_used.load(Ordering::Relaxed).then_some(self.trash_dir.as_path())
    }

    /// Removes a file or folder from the destination.
    pub fn dispose(&self, path: &Path, rel: &RelPath) -> io::Result<()> {
        let is_dir = fs::symlink_metadata(path)?.is_dir();
        if !is_dir {
            files::clear_readonly(path)?;
        }
        match self.mode {
            DeleteMode::Permanent if is_dir => files::remove_dir_all_force(path),
            DeleteMode::Permanent => fs::remove_file(path),
            DeleteMode::TrashFolder => self.move_to_trash(path, rel),
            DeleteMode::RecycleBin => match trash::delete(plain_path(path)) {
                Ok(()) => Ok(()),
                Err(err) => {
                    self.note_fallback(&err.to_string());
                    self.move_to_trash(path, rel)
                }
            },
        }
    }

    fn note_fallback(&self, error: &str) {
        let mut fallback = self.fallback.lock().unwrap();
        if fallback.is_none() {
            *fallback = Some(format!(
                "Recycle Bin failed for some files ({error}); they were moved to {}",
                display_path(&self.trash_dir)
            ));
        }
    }

    fn move_to_trash(&self, path: &Path, rel: &RelPath) -> io::Result<()> {
        let _guard = self.trash_lock.lock().unwrap();
        let base = rel.to_native(&self.trash_dir);
        if let Some(parent) = base.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut target = base.clone();
        let mut counter = 1;
        while fs::symlink_metadata(&target).is_ok() {
            let mut name = base.as_os_str().to_os_string();
            name.push(format!(".{counter}"));
            target = PathBuf::from(name);
            counter += 1;
        }
        fs::rename(path, &target)?;
        self.trash_used.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Keeps only the `keep` most recent run folders in `<destination>/.mirror-trash`.
/// Returns how many were removed.
pub fn prune_trash(destination_root: &Path, keep: usize) -> io::Result<usize> {
    let trash = destination_root.join(TRASH_DIR_NAME);
    let entries = match fs::read_dir(&trash) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut runs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    // Run folders are named by date (`2026-10-08_143000`), so name order is time order.
    runs.sort();
    let excess = runs.len().saturating_sub(keep);
    for dir in &runs[..excess] {
        files::remove_dir_all_force(dir)?;
    }
    Ok(excess)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn falls_back_to_the_trash_folder_on_network_paths() {
        let deleter = Deleter::new(DeleteMode::RecycleBin, Path::new(r"\\server\share\mirror"), "run");
        assert_eq!(deleter.effective_mode(), DeleteMode::TrashFolder);
        assert!(deleter.fallback_note().unwrap().contains("network path"));
    }

    #[test]
    fn other_modes_are_used_as_requested() {
        let root = Path::new("anything");
        assert_eq!(Deleter::new(DeleteMode::Permanent, root, "run").effective_mode(), DeleteMode::Permanent);
        assert_eq!(Deleter::new(DeleteMode::TrashFolder, root, "run").effective_mode(), DeleteMode::TrashFolder);
        assert!(Deleter::new(DeleteMode::TrashFolder, root, "run").fallback_note().is_none());
    }

    #[test]
    fn the_same_name_twice_in_one_run_gets_a_numbered_copy() {
        let dir = tempfile::tempdir().unwrap();
        let deleter = Deleter::new(DeleteMode::TrashFolder, dir.path(), "run");
        let file = dir.path().join("a.txt");
        let rel = RelPath::new("a.txt");
        for content in ["first", "second"] {
            fs::write(&file, content).unwrap();
            deleter.dispose(&file, &rel).unwrap();
        }

        let trash = dir.path().join(TRASH_DIR_NAME).join("run");
        assert_eq!(fs::read_to_string(trash.join("a.txt")).unwrap(), "first");
        assert_eq!(fs::read_to_string(trash.join("a.txt.1")).unwrap(), "second");
        assert_eq!(deleter.trash_dir_used(), Some(trash.as_path()));
    }

    #[test]
    fn whole_folders_can_be_disposed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("old");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("x.txt"), "x").unwrap();

        Deleter::new(DeleteMode::Permanent, dir.path(), "run").dispose(&folder, &RelPath::new("old")).unwrap();

        assert!(!folder.exists());
    }

    #[test]
    fn prune_without_a_trash_folder_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(prune_trash(dir.path(), 1).unwrap(), 0);
    }
}
