//! File operations with retries: safe copy (temp file + rename), case renames.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use super::deleter::Deleter;
use crate::shared::error::{classify_io_error, ProblemReason};
use crate::shared::files::{self, TEMP_SUFFIX};
use crate::shared::hashing;
use crate::shared::paths::RelPath;

/// Failure of a single attempt.
#[derive(Debug)]
pub struct Failure {
    pub reason: ProblemReason,
    pub error: String,
}

impl Failure {
    pub fn new(reason: ProblemReason, error: impl Into<String>) -> Self {
        Self { reason, error: error.into() }
    }
}

impl From<io::Error> for Failure {
    fn from(err: io::Error) -> Self {
        Self::new(classify_io_error(&err), err.to_string())
    }
}

/// Final failure of an action.
#[derive(Debug)]
pub struct OpError {
    pub reason: ProblemReason,
    pub error: String,
    pub attempts: u32,
}

impl OpError {
    pub fn new(reason: ProblemReason, error: impl Into<String>, attempts: u32) -> Self {
        Self { reason, error: error.into(), attempts }
    }
}

/// Retry policy for temporary problems (locked files, files changing while copied).
pub struct Retry {
    pub attempts: u32,
    pub delay: Duration,
}

impl Retry {
    /// Runs `op` until it succeeds, fails with a permanent problem, or attempts run out.
    /// The wait grows with each attempt. Returns the number of attempts used.
    pub fn run(&self, cancel: &AtomicBool, mut op: impl FnMut() -> Result<(), Failure>) -> Result<u32, OpError> {
        let max = self.attempts.max(1);
        let mut attempt = 1;
        loop {
            match op() {
                Ok(()) => return Ok(attempt),
                Err(failure)
                    if failure.reason.is_retryable() && attempt < max && !cancel.load(Ordering::Relaxed) =>
                {
                    thread::sleep(self.delay * attempt);
                    attempt += 1;
                }
                Err(failure) => return Err(OpError::new(failure.reason, failure.error, attempt)),
            }
        }
    }
}

/// Copies `source` over `destination` safely: the data goes to a temporary file next to the
/// destination, which replaces the real file only when complete. An interrupted copy never
/// leaves a half-written file. The old version is handed to `backup` when given.
pub fn copy_file(
    source: &Path,
    destination: &Path,
    rel: &RelPath,
    backup: Option<&Deleter>,
    verify: bool,
) -> Result<(), Failure> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = temp_path(destination);
    files::remove_quietly(&temp);
    let result = copy_to_temp(source, &temp, verify).and_then(|()| replace(&temp, destination, rel, backup));
    if result.is_err() {
        files::remove_quietly(&temp);
    }
    result
}

fn copy_to_temp(source: &Path, temp: &Path, verify: bool) -> Result<(), Failure> {
    let before = fs::metadata(source)?;
    fs::copy(source, temp)?;
    let after = fs::metadata(source)?;
    if changed(&before, &after) {
        return Err(Failure::new(
            ProblemReason::ChangedDuringCopy,
            "the source file changed while it was being copied",
        ));
    }
    if fs::metadata(temp)?.len() != after.len() {
        return Err(Failure::new(ProblemReason::VerifyFailed, "the copied size does not match the source"));
    }
    files::set_modified(temp, after.modified()?)?;
    if verify {
        let (a, b) = rayon::join(|| hashing::full_hash(source), || hashing::full_hash(temp));
        if a? != b? {
            return Err(Failure::new(ProblemReason::VerifyFailed, "the copied content does not match the source"));
        }
    }
    Ok(())
}

fn replace(temp: &Path, destination: &Path, rel: &RelPath, backup: Option<&Deleter>) -> Result<(), Failure> {
    match fs::symlink_metadata(destination) {
        Ok(meta) if meta.is_dir() => {
            return Err(Failure::new(ProblemReason::IoError, "a folder exists at the destination path"));
        }
        Ok(meta) if meta.is_file() => match backup {
            Some(deleter) => deleter.dispose(destination, rel)?,
            None => files::clear_readonly(destination)?,
        },
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    fs::rename(temp, destination)?;
    Ok(())
}

/// Changes only the letter case of a name, through a temporary name so it also works on
/// case-insensitive file systems.
pub fn rename_case(from: &Path, to: &Path) -> Result<(), Failure> {
    let temp = with_suffix(to, ".mirror-case-tmp");
    fs::rename(from, &temp)?;
    if let Err(err) = fs::rename(&temp, to) {
        let _ = fs::rename(&temp, from);
        return Err(err.into());
    }
    Ok(())
}

fn changed(before: &Metadata, after: &Metadata) -> bool {
    before.len() != after.len() || before.modified().ok() != after.modified().ok()
}

fn temp_path(destination: &Path) -> PathBuf {
    with_suffix(destination, TEMP_SUFFIX)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
