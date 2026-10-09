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
/// `on_progress` receives the bytes copied so far for this file.
pub fn copy_file(
    source: &Path,
    destination: &Path,
    rel: &RelPath,
    backup: Option<&Deleter>,
    verify: bool,
    on_progress: &dyn Fn(u64),
) -> Result<(), Failure> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = temp_path(destination);
    files::remove_quietly(&temp);
    let result = copy_to_temp(source, &temp, verify, on_progress)
        .and_then(|()| replace(&temp, destination, rel, backup));
    if result.is_err() {
        files::remove_quietly(&temp);
    }
    result
}

fn copy_to_temp(source: &Path, temp: &Path, verify: bool, on_progress: &dyn Fn(u64)) -> Result<(), Failure> {
    let before = fs::metadata(source)?;
    copy_with_progress(source, temp, on_progress)?;
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

/// Copies a file like `fs::copy` (data, attributes / permissions), reporting the bytes copied
/// so far. On Windows this is the native `CopyFileExW` with its progress callback.
#[cfg(windows)]
fn copy_with_progress(source: &Path, target: &Path, on_progress: &dyn Fn(u64)) -> io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{CopyFileExW, LPPROGRESS_ROUTINE_CALLBACK_REASON};

    const PROGRESS_CONTINUE: u32 = 0;

    unsafe extern "system" fn callback(
        _total_size: i64,
        transferred: i64,
        _stream_size: i64,
        _stream_transferred: i64,
        _stream_number: u32,
        _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
        _source: HANDLE,
        _target: HANDLE,
        data: *const c_void,
    ) -> u32 {
        // SAFETY: `data` is the `&&dyn Fn(u64)` passed to CopyFileExW below, which outlives the call.
        let on_progress = unsafe { &*(data as *const &dyn Fn(u64)) };
        on_progress(transferred.max(0) as u64);
        PROGRESS_CONTINUE
    }

    let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (from, to) = (wide(source), wide(target));
    let data: *const &dyn Fn(u64) = &on_progress;
    // SAFETY: both paths are NUL-terminated UTF-16 buffers alive for the call, and `data`
    // points to a live closure reference; the callback only reads it.
    let ok = unsafe {
        CopyFileExW(from.as_ptr(), to.as_ptr(), Some(callback), data as *const c_void, std::ptr::null_mut(), 0)
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn copy_with_progress(source: &Path, target: &Path, on_progress: &dyn Fn(u64)) -> io::Result<()> {
    use std::io::{Read, Write};

    let mut input = fs::File::open(source)?;
    let mut output = fs::File::create(target)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut copied = 0u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        copied += read as u64;
        on_progress(copied);
    }
    output.flush()?;
    fs::set_permissions(target, input.metadata()?.permissions())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn retry() -> Retry {
        Retry { attempts: 3, delay: Duration::from_millis(1) }
    }

    fn fail(reason: ProblemReason) -> Result<(), Failure> {
        Err(Failure::new(reason, "test"))
    }

    #[test]
    fn retries_locked_files_until_they_succeed() {
        let calls = Cell::new(0);
        let result = retry().run(&AtomicBool::new(false), || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 { fail(ProblemReason::FileLocked) } else { Ok(()) }
        });
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn gives_up_after_the_last_attempt() {
        let error = retry().run(&AtomicBool::new(false), || fail(ProblemReason::FileLocked)).unwrap_err();
        assert_eq!((error.reason, error.attempts), (ProblemReason::FileLocked, 3));
    }

    #[test]
    fn does_not_retry_permanent_problems_or_when_cancelled() {
        let denied = retry().run(&AtomicBool::new(false), || fail(ProblemReason::PermissionDenied));
        assert_eq!(denied.unwrap_err().attempts, 1);
        let cancelled = retry().run(&AtomicBool::new(true), || fail(ProblemReason::FileLocked));
        assert_eq!(cancelled.unwrap_err().attempts, 1);
    }

    #[test]
    fn copy_reports_progress_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let destination = dir.path().join("out").join("copy.bin");
        let data = vec![42u8; 3 * 1024 * 1024];
        fs::write(&source, &data).unwrap();
        let last = Cell::new(0u64);

        copy_file(&source, &destination, &RelPath::new("copy.bin"), None, true, &|copied| last.set(copied)).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), data);
        assert_eq!(last.get(), data.len() as u64);
        assert!(!temp_path(&destination).exists());
    }

    #[test]
    fn rename_case_changes_only_the_letter_case() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("report.txt");
        fs::write(&from, "x").unwrap();

        rename_case(&from, &dir.path().join("Report.txt")).unwrap();

        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["Report.txt"]);
    }
}
