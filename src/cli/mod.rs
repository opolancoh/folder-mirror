//! Command-line interface. Only parses input, calls the `app` use cases and prints results.

mod args;
mod commands;
mod output;
mod progress;

pub use args::Cli;
pub use commands::run;
