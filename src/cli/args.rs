use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use folder_mirror::features::report::ReportFormat;
use folder_mirror::shared::config::{to_duration, CompareMode, DeleteMode, Settings};
use folder_mirror::shared::error::Result;

#[derive(Debug, Parser)]
#[command(
    name = "mirror",
    version,
    about = "Keep a mirror folder identical to its source folder",
    after_help = "Exit codes: 0 in sync / success, 1 differences found (analyze), 2 completed with problems, 3 aborted"
)]
pub struct Cli {
    /// Without a command, shows how the tool works and the default settings.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Compare source and destination and write the result file. Changes nothing.
    Analyze(AnalyzeArgs),
    /// Analyze, show the summary, ask for confirmation, then make the destination identical.
    Sync(SyncArgs),
}

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    #[command(flatten)]
    pub options: OptionArgs,
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    #[command(flatten)]
    pub options: OptionArgs,
    #[command(flatten)]
    pub output: OutputArgs,
    /// Execute a plan written by `analyze` (JSON) instead of analyzing again
    #[arg(long, value_name = "FILE", conflicts_with_all = ["source", "destination"])]
    pub plan: Option<PathBuf>,
    /// Do not ask for confirmation (for scheduled runs)
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct TargetArgs {
    /// Source folder (the one that is always right)
    #[arg(short, long, value_name = "DIR")]
    pub source: Option<PathBuf>,
    /// Destination folder (the mirror)
    #[arg(short, long, value_name = "DIR")]
    pub destination: Option<PathBuf>,
    /// Use a profile from the config file
    #[arg(short, long, value_name = "NAME")]
    pub profile: Option<String>,
    /// Config file with profiles [default: mirror.toml]
    #[arg(long, value_name = "FILE", requires = "profile")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct OptionArgs {
    /// Comparison mode: name, metadata, smart, quick-hash, full-hash [default: smart]
    #[arg(short, long, value_name = "MODE")]
    pub mode: Option<CompareMode>,
    /// Allowed difference between modification times, in seconds [default: 2]
    #[arg(long, value_name = "SECONDS")]
    pub mtime_tolerance: Option<f64>,
    /// Exclude names or paths matching a glob pattern (repeatable). Without '/' it matches names at any depth
    #[arg(short = 'x', long = "exclude", value_name = "PATTERN")]
    pub exclude: Vec<String>,
    /// Do not exclude OS files (Thumbs.db, .DS_Store, $RECYCLE.BIN...)
    #[arg(long)]
    pub no_default_excludes: bool,
    /// Where removed files go: recycle-bin, trash-folder, permanent [default: recycle-bin]
    #[arg(long, value_name = "MODE")]
    pub delete_mode: Option<DeleteMode>,
    /// Number of runs kept in the .mirror-trash folder [default: 4]
    #[arg(long, value_name = "N")]
    pub trash_keep: Option<usize>,
    /// Do not keep the old version of overwritten files
    #[arg(long)]
    pub no_backup_overwritten: bool,
    /// Block the sync if more than this % of destination files would be deleted [default: 20]
    #[arg(long, value_name = "PERCENT")]
    pub max_delete: Option<f64>,
    /// Attempts for locked or changing files [default: 3]
    #[arg(long, value_name = "N")]
    pub retries: Option<u32>,
    /// Base wait between attempts, in seconds; grows with each attempt [default: 2]
    #[arg(long, value_name = "SECONDS")]
    pub retry_delay: Option<f64>,
    /// Hash every copied file to confirm it matches the source
    #[arg(long)]
    pub verify: bool,
    /// Stop at the first failed file instead of continuing
    #[arg(long)]
    pub stop_on_error: bool,
    /// Worker threads [default: number of CPUs]
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
    /// List problems with their paths up to this count; above it, show counts by reason [default: 7]
    #[arg(long, value_name = "N")]
    pub max_errors_shown: Option<usize>,
}

impl OptionArgs {
    /// Command-line values override the profile and defaults.
    pub fn apply(&self, settings: &mut Settings) -> Result<()> {
        if let Some(mode) = self.mode {
            settings.mode = mode;
        }
        if let Some(seconds) = self.mtime_tolerance {
            settings.mtime_tolerance = to_duration(seconds, "--mtime-tolerance")?;
        }
        settings.excludes.extend(self.exclude.iter().cloned());
        if self.no_default_excludes {
            settings.default_excludes = false;
        }
        if let Some(mode) = self.delete_mode {
            settings.delete_mode = mode;
        }
        if let Some(keep) = self.trash_keep {
            settings.trash_keep = keep;
        }
        if self.no_backup_overwritten {
            settings.backup_overwritten = false;
        }
        if let Some(percent) = self.max_delete {
            settings.max_delete_percent = percent;
        }
        if let Some(retries) = self.retries {
            settings.retries = retries;
        }
        if let Some(seconds) = self.retry_delay {
            settings.retry_delay = to_duration(seconds, "--retry-delay")?;
        }
        if self.verify {
            settings.verify = true;
        }
        if self.stop_on_error {
            settings.stop_on_error = true;
        }
        if self.threads.is_some() {
            settings.threads = self.threads;
        }
        if let Some(count) = self.max_errors_shown {
            settings.max_errors_shown = count;
        }
        Ok(())
    }
}

#[derive(Debug, Args)]
pub struct OutputArgs {
    /// Result file path [default: mirror-result-<date>.<format> in the current folder]
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Result file format: json, csv [default: from the file extension, else json]
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<ReportFormat>,
}
