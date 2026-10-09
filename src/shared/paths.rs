//! Path helpers: portable relative paths, root resolution, display and case sensitivity.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A path relative to a sync root, always stored with `/` separators so it is the same on every OS.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelPath(String);

impl RelPath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into().replace('\\', "/").trim_matches('/').to_string())
    }

    /// Builds a relative path from native components. Returns `None` for names that are not valid
    /// Unicode or for components such as `..`.
    pub fn from_relative(path: &Path) -> Option<Self> {
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(name) => parts.push(name.to_str()?),
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(Self(parts.join("/")))
    }

    /// Path of `full` relative to `root`.
    pub fn from_root(root: &Path, full: &Path) -> Option<Self> {
        full.strip_prefix(root).ok().and_then(Self::from_relative)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Last component (file or folder name).
    pub fn name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }

    /// Number of components: `a` is 1, `a/b` is 2.
    pub fn depth(&self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            self.0.matches('/').count() + 1
        }
    }

    pub fn to_native(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        for segment in self.0.split('/').filter(|s| !s.is_empty()) {
            path.push(segment);
        }
        path
    }

    /// Key used to match entries between both sides.
    pub fn key(&self, case_insensitive: bool) -> String {
        if case_insensitive {
            self.0.to_lowercase()
        } else {
            self.0.clone()
        }
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// All ancestor keys of a key: `a/b/c` gives `a` and `a/b`.
pub fn ancestor_keys(key: &str) -> impl Iterator<Item = &str> {
    key.match_indices('/').map(move |(index, _)| &key[..index])
}

/// Human-readable path, without the Windows `\\?\` long-path prefix.
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text.into_owned()
    }
}

/// The same path without the long-path prefix, for APIs that do not accept it.
pub fn plain_path(path: &Path) -> PathBuf {
    PathBuf::from(display_path(path))
}

/// Absolute, normalized form of a root folder. On Windows this returns a `\\?\` path, which
/// also lifts the 260-character limit for everything joined below it. Works for folders that do
/// not exist yet by resolving the closest existing ancestor.
pub fn resolve_root(path: &Path) -> io::Result<PathBuf> {
    if let Ok(resolved) = fs::canonicalize(path) {
        return Ok(resolved);
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut missing = Vec::new();
    let mut current = absolute.as_path();
    loop {
        match fs::canonicalize(current) {
            Ok(mut base) => {
                for name in missing.iter().rev() {
                    base.push(name);
                }
                return Ok(base);
            }
            Err(err) => match (current.parent(), current.file_name()) {
                (Some(parent), Some(name)) => {
                    missing.push(name.to_os_string());
                    current = parent;
                }
                _ => return Err(err),
            },
        }
    }
}

/// True when one root is inside the other (or both are the same).
pub fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Detects whether names under `root` are case-insensitive, by looking up the root's own name
/// with swapped letter case. Falls back to the platform default when it cannot tell.
pub fn detect_case_insensitive(root: &Path) -> bool {
    let default = cfg!(any(windows, target_os = "macos"));
    let (Some(parent), Some(name)) = (root.parent(), root.file_name().and_then(|n| n.to_str()))
    else {
        return default;
    };
    let swapped: String = name
        .chars()
        .map(|c| {
            if c.is_lowercase() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        })
        .collect();
    if swapped == name || !root.is_dir() {
        return default;
    }
    fs::metadata(parent.join(swapped)).is_ok()
}

/// Why a file name cannot be stored on Windows, if it cannot.
pub fn windows_name_issue(name: &str) -> Option<&'static str> {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || (c as u32) < 32)
    {
        return Some("contains characters not allowed on Windows");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("ends with a dot or a space");
    }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Some("is a reserved name on Windows");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_path_is_normalized() {
        let path = RelPath::new("\\docs\\a.txt/");
        assert_eq!(path.as_str(), "docs/a.txt");
        assert_eq!(path.name(), "a.txt");
        assert_eq!(path.depth(), 2);
        assert!(RelPath::new("").is_root());
        assert_eq!(RelPath::new("").depth(), 0);
    }

    #[test]
    fn rel_path_from_relative_skips_dots_and_rejects_parents() {
        assert_eq!(RelPath::from_relative(Path::new("a/./b")).unwrap().as_str(), "a/b");
        assert!(RelPath::from_relative(Path::new("../x")).is_none());
    }

    #[test]
    fn rel_path_from_root_strips_the_root() {
        let root = Path::new("root");
        assert_eq!(RelPath::from_root(root, &root.join("a").join("b")).unwrap().as_str(), "a/b");
        assert!(RelPath::from_root(root, Path::new("other/a")).is_none());
    }

    #[test]
    fn key_folds_case_only_when_insensitive() {
        let path = RelPath::new("Docs/Report.TXT");
        assert_eq!(path.key(true), "docs/report.txt");
        assert_eq!(path.key(false), "Docs/Report.TXT");
    }

    #[test]
    fn to_native_joins_each_segment() {
        let native = RelPath::new("a/b.txt").to_native(Path::new("root"));
        assert_eq!(native, Path::new("root").join("a").join("b.txt"));
    }

    #[test]
    fn ancestor_keys_lists_every_parent() {
        assert_eq!(ancestor_keys("a/b/c").collect::<Vec<_>>(), ["a", "a/b"]);
        assert_eq!(ancestor_keys("a").count(), 0);
    }

    #[test]
    fn display_path_removes_the_long_path_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\data")), r"C:\data");
        assert_eq!(display_path(Path::new(r"\\?\UNC\server\share")), r"\\server\share");
        assert_eq!(display_path(Path::new("/home/user")), "/home/user");
    }

    #[test]
    fn windows_name_issue_detects_names_windows_cannot_store() {
        assert!(windows_name_issue("a:b.txt").is_some());
        assert!(windows_name_issue("file.").is_some());
        assert!(windows_name_issue("file ").is_some());
        assert!(windows_name_issue("CON").is_some());
        assert!(windows_name_issue("nul.txt").is_some());
        assert!(windows_name_issue("normal.txt").is_none());
        assert!(windows_name_issue("CONSOLE.txt").is_none());
    }

    #[test]
    fn overlaps_compares_whole_components() {
        assert!(overlaps(Path::new("/x"), Path::new("/x")));
        assert!(overlaps(Path::new("/x"), Path::new("/x/y")));
        assert!(overlaps(Path::new("/x/y"), Path::new("/x")));
        assert!(!overlaps(Path::new("/x"), Path::new("/xy")));
    }

    #[test]
    fn resolve_root_works_for_folders_that_do_not_exist_yet() {
        let missing = std::env::temp_dir().join("folder-mirror-test-missing").join("child");
        let resolved = resolve_root(&missing).unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with(Path::new("folder-mirror-test-missing").join("child")));
    }
}
