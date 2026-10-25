//! Arguments for `kbshff ui`, a launcher for the separate `kbshff-studio` app.

use std::path::PathBuf;

use clap::Parser;

/// Arguments forwarded to `kbshff-studio` (operator studio; not an orchestrator entrypoint).
#[derive(Debug, Clone, Parser)]
pub struct UiArgs {
    /// Project root that owns `.kuibysheff/`.
    #[arg(long, value_name = "DIR")]
    pub project_root: PathBuf,

    /// Listen address (loopback only). Default: `127.0.0.1:8790`.
    #[arg(long, value_name = "ADDR", default_value = "127.0.0.1:8790")]
    pub bind: String,

    /// Open the studio URL in the default browser after bind.
    #[arg(long)]
    pub open: bool,
}
