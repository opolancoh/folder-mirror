//! Exclusion rules: built-in OS files plus user glob patterns. Applied to both sides, so excluded
//! entries are never copied and never deleted.

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use crate::shared::error::{MirrorError, Result};
use crate::shared::paths::RelPath;

/// OS system and metadata files that should never be mirrored.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    // Windows
    "System Volume Information",
    "$RECYCLE.BIN",
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
    "Thumbs.db",
    "desktop.ini",
    // macOS
    ".DS_Store",
    "._*",
    ".Spotlight-V100",
    ".Trashes",
    ".fseventsd",
    ".TemporaryItems",
    // Linux
    "lost+found",
    ".Trash-*",
];

pub struct Excludes {
    /// Patterns without `/`: matched against the file or folder name at any depth.
    names: GlobSet,
    /// Patterns with `/`: matched against the whole relative path.
    paths: GlobSet,
}

impl Excludes {
    pub fn new(patterns: &[String], use_defaults: bool) -> Result<Self> {
        let mut names = GlobSetBuilder::new();
        let mut paths = GlobSetBuilder::new();

        if use_defaults {
            for pattern in DEFAULT_EXCLUDES {
                names.add(build(GlobBuilder::new(pattern).case_insensitive(true), pattern)?);
            }
        }
        for raw in patterns {
            let pattern = raw.trim().trim_start_matches("./").trim_matches('/');
            if pattern.is_empty() {
                continue;
            }
            if pattern.contains('/') {
                paths.add(build(GlobBuilder::new(pattern).literal_separator(true), raw)?);
            } else {
                names.add(Glob::new(pattern).map_err(|e| invalid(raw, e))?);
            }
        }

        Ok(Self {
            names: names.build().map_err(|e| invalid("exclude list", e))?,
            paths: paths.build().map_err(|e| invalid("exclude list", e))?,
        })
    }

    pub fn is_excluded(&self, path: &RelPath) -> bool {
        self.names.is_match(path.name()) || self.paths.is_match(path.as_str())
    }
}

fn build(builder: &mut GlobBuilder, pattern: &str) -> Result<Glob> {
    builder.build().map_err(|e| invalid(pattern, e))
}

fn invalid(pattern: &str, err: globset::Error) -> MirrorError {
    MirrorError::Config(format!("invalid exclude pattern '{pattern}': {err}"))
}
