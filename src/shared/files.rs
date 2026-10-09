//! Low-level file helpers: timestamps, read-only flags, free space and Recycle Bin availability.

use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};

use super::paths::plain_path;

/// Folder inside the destination root where removed files go in `trash-folder` mode.
pub const TRASH_DIR_NAME: &str = ".mirror-trash";

/// Suffix of the temporary file written before it replaces the real one.
pub const TEMP_SUFFIX: &str = ".mirror-tmp";

/// Sets the modification time of a file (also works on read-only files).
pub fn set_modified(path: &Path, time: SystemTime) -> io::Result<()> {
    open_for_attributes(path)?.set_modified(time)
}

#[cfg(windows)]
fn open_for_attributes(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .open(path)
}

#[cfg(not(windows))]
fn open_for_attributes(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Removes the read-only attribute so the file can be replaced or deleted. Only needed on
/// Windows; on Unix, deleting depends on the folder permissions.
pub fn clear_readonly(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut permissions = fs::symlink_metadata(path)?.permissions();
        if permissions.readonly() {
            // Windows-only code: here it just clears the read-only attribute.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

/// Deletes a file if possible, ignoring errors. Used to clean temporary files.
pub fn remove_quietly(path: &Path) {
    let _ = clear_readonly(path);
    let _ = fs::remove_file(path);
}

/// Deletes a folder and everything inside, including read-only files.
pub fn remove_dir_all_force(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
        if entry.file_type().is_file() {
            let _ = clear_readonly(entry.path());
        }
    }
    fs::remove_dir_all(path)
}

/// Free space for the volume holding `path` (or its closest existing ancestor).
pub fn available_space(path: &Path) -> io::Result<u64> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return fs2::available_space(plain_path(candidate));
        }
        current = candidate.parent();
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no existing folder to measure free space",
    ))
}

/// Exact, readable timestamp used in the result file (RFC 3339 with nanoseconds).
pub fn time_to_string(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Nanos, true)
}

pub fn time_from_string(text: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(text).ok().map(SystemTime::from)
}

/// True when both times differ by at most `tolerance`.
pub fn times_match(a: SystemTime, b: SystemTime, tolerance: Duration) -> bool {
    let difference = match a.duration_since(b) {
        Ok(d) => d,
        Err(e) => e.duration(),
    };
    difference <= tolerance
}

/// Returns why the OS Recycle Bin cannot be used for `path`, or `None` if it can.
/// On Windows, network and removable drives have no Recycle Bin: deleting there is permanent.
#[cfg(windows)]
pub fn recycle_bin_unavailable_reason(path: &Path) -> Option<&'static str> {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_REMOTE: u32 = 4;

    let letter = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => return Some("network path"),
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => return None,
        },
        _ => return None,
    };
    let root: Vec<u16> = format!("{}:\\", letter as char)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: `root` is a valid, NUL-terminated UTF-16 string that outlives the call.
    match unsafe { GetDriveTypeW(root.as_ptr()) } {
        DRIVE_REMOTE => Some("network drive"),
        DRIVE_REMOVABLE => Some("removable drive"),
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn recycle_bin_unavailable_reason(_path: &Path) -> Option<&'static str> {
    None
}
