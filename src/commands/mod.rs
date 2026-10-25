//! Management CLI commands (human-readable I/O; not `RunOutput` JSON).

pub mod agent;
pub mod check;
pub mod config;
pub mod dotenv_file;
pub mod init;
pub mod mcp_probe;
pub mod prompt;
pub mod wizard;

use std::io::{self, Write};

/// Write one pretty-printed JSON document to stdout.
///
/// # Errors
///
/// Returns I/O errors from stdout.
pub fn print_json(value: &impl serde::Serialize) -> io::Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}
