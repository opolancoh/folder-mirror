//! Settings for a run, the option enums, and the optional `mirror.toml` profiles file.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::error::{MirrorError, Result};

/// How files that exist on both sides are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompareMode {
    /// Only presence: finds added and removed files, never modified ones.
    Name,
    /// Size and modification time.
    Metadata,
    /// Size and time; when only the time differs, a full hash confirms the change.
    #[default]
    Smart,
    /// Size plus a hash of three sampled blocks.
    QuickHash,
    /// Size plus a hash of the whole file.
    FullHash,
}

impl CompareMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Metadata => "metadata",
            Self::Smart => "smart",
            Self::QuickHash => "quick-hash",
            Self::FullHash => "full-hash",
        }
    }
}

impl FromStr for CompareMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "name" => Ok(Self::Name),
            "metadata" => Ok(Self::Metadata),
            "smart" => Ok(Self::Smart),
            "quick-hash" | "quick" => Ok(Self::QuickHash),
            "full-hash" | "full" => Ok(Self::FullHash),
            _ => Err(format!(
                "unknown mode '{value}' (expected: name, metadata, smart, quick-hash, full-hash)"
            )),
        }
    }
}

impl fmt::Display for CompareMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happens to files removed from the destination (and to old versions of overwritten files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMode {
    /// OS Recycle Bin / Trash, falling back to the trash folder where it is not available.
    #[default]
    RecycleBin,
    /// `<destination>/.mirror-trash/<date>/`.
    TrashFolder,
    /// Deleted for good.
    Permanent,
}

impl DeleteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RecycleBin => "recycle-bin",
            Self::TrashFolder => "trash-folder",
            Self::Permanent => "permanent",
        }
    }
}

impl FromStr for DeleteMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "recycle-bin" | "recycle" | "bin" => Ok(Self::RecycleBin),
            "trash-folder" | "trash" => Ok(Self::TrashFolder),
            "permanent" => Ok(Self::Permanent),
            _ => Err(format!(
                "unknown delete mode '{value}' (expected: recycle-bin, trash-folder, permanent)"
            )),
        }
    }
}

impl fmt::Display for DeleteMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every option of a run, already merged from defaults, profile and command line.
#[derive(Debug, Clone)]
pub struct Settings {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub mode: CompareMode,
    pub mtime_tolerance: Duration,
    pub excludes: Vec<String>,
    pub default_excludes: bool,
    pub delete_mode: DeleteMode,
    pub trash_keep: usize,
    pub backup_overwritten: bool,
    pub max_delete_percent: f64,
    pub retries: u32,
    pub retry_delay: Duration,
    pub verify: bool,
    pub stop_on_error: bool,
    pub threads: Option<usize>,
    pub max_errors_shown: usize,
}

impl Settings {
    pub fn new(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            mode: CompareMode::default(),
            mtime_tolerance: Duration::from_secs(2),
            excludes: Vec::new(),
            default_excludes: true,
            delete_mode: DeleteMode::default(),
            trash_keep: 4,
            backup_overwritten: true,
            max_delete_percent: 20.0,
            retries: 3,
            retry_delay: Duration::from_secs(2),
            verify: false,
            stop_on_error: false,
            threads: None,
            max_errors_shown: 7,
        }
    }

    /// Applies the values set in a profile. Source and destination are resolved by the caller.
    pub fn apply_profile(&mut self, profile: &Profile) -> Result<()> {
        if let Some(mode) = profile.mode {
            self.mode = mode;
        }
        if let Some(seconds) = profile.mtime_tolerance {
            self.mtime_tolerance = to_duration(seconds, "mtime_tolerance")?;
        }
        self.excludes.extend(profile.exclude.iter().cloned());
        if let Some(value) = profile.default_excludes {
            self.default_excludes = value;
        }
        if let Some(mode) = profile.delete_mode {
            self.delete_mode = mode;
        }
        if let Some(keep) = profile.trash_keep {
            self.trash_keep = keep;
        }
        if let Some(value) = profile.backup_overwritten {
            self.backup_overwritten = value;
        }
        if let Some(percent) = profile.max_delete {
            self.max_delete_percent = percent;
        }
        if let Some(retries) = profile.retries {
            self.retries = retries;
        }
        if let Some(seconds) = profile.retry_delay {
            self.retry_delay = to_duration(seconds, "retry_delay")?;
        }
        if let Some(value) = profile.verify {
            self.verify = value;
        }
        if let Some(value) = profile.stop_on_error {
            self.stop_on_error = value;
        }
        if profile.threads.is_some() {
            self.threads = profile.threads;
        }
        if let Some(count) = profile.max_errors_shown {
            self.max_errors_shown = count;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if !(0.0..=100.0).contains(&self.max_delete_percent) {
            return Err(MirrorError::Config("max-delete must be between 0 and 100".into()));
        }
        if self.retries == 0 {
            return Err(MirrorError::Config("retries must be at least 1".into()));
        }
        if self.trash_keep == 0 {
            return Err(MirrorError::Config("trash-keep must be at least 1".into()));
        }
        if self.threads == Some(0) {
            return Err(MirrorError::Config("threads must be at least 1".into()));
        }
        Ok(())
    }
}

/// Converts seconds from the command line or the config file into a duration.
pub fn to_duration(seconds: f64, name: &str) -> Result<Duration> {
    Duration::try_from_secs_f64(seconds)
        .map_err(|_| MirrorError::Config(format!("{name} must be zero or a positive number of seconds")))
}

/// One named profile in `mirror.toml`. Every field is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub mode: Option<CompareMode>,
    /// Seconds.
    pub mtime_tolerance: Option<f64>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub default_excludes: Option<bool>,
    pub delete_mode: Option<DeleteMode>,
    pub trash_keep: Option<usize>,
    pub backup_overwritten: Option<bool>,
    /// Percent.
    pub max_delete: Option<f64>,
    pub retries: Option<u32>,
    /// Seconds.
    pub retry_delay: Option<f64>,
    pub verify: Option<bool>,
    pub stop_on_error: Option<bool>,
    pub threads: Option<usize>,
    pub max_errors_shown: Option<usize>,
}

/// The `mirror.toml` file: `[profiles.<name>]` tables.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl ConfigFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .map_err(|e| MirrorError::Config(format!("cannot read {}: {e}", path.display())))?;
        toml::from_str(&text)
            .map_err(|e| MirrorError::Config(format!("invalid {}: {e}", path.display())))
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).ok_or_else(|| {
            let available: Vec<&str> = self.profiles.keys().map(String::as_str).collect();
            MirrorError::Config(format!(
                "profile '{name}' not found (available: {})",
                if available.is_empty() { "none".to_string() } else { available.join(", ") }
            ))
        })
    }
}
