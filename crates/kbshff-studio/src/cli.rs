//! Arguments for the `kbshff-studio` process.

use std::path::PathBuf;

use clap::Parser;

/// Local browser studio. Agent work is done by the `kbshff` CLI.
#[derive(Debug, Parser)]
#[command(name = "kbshff-studio", version, about)]
pub struct StudioArgs {
    /// Project root that owns `.kuibysheff/`. Defaults to the current directory.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub project_root: PathBuf,

    /// Listen address (loopback only). Default: `127.0.0.1:8790`.
    #[arg(long, value_name = "ADDR", default_value = "127.0.0.1:8790")]
    pub bind: String,

    /// Open the studio URL in the default browser after bind.
    ///
    /// This is the default. Pass `--no-open` to leave the browser closed.
    #[arg(long, conflicts_with = "no_open")]
    pub open: bool,

    /// Do not open a browser after bind.
    #[arg(long, conflicts_with = "open")]
    pub no_open: bool,

    /// Path to the `kbshff` binary. Overrides `KBSHFF_BIN` and a sibling lookup.
    #[arg(long, value_name = "PATH")]
    pub kbshff: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::StudioArgs;
    use clap::Parser;

    #[test]
    fn bare_command_uses_cwd_and_opens_browser() {
        let args = StudioArgs::try_parse_from(["kbshff-studio"]).expect("parse");
        assert_eq!(args.project_root.as_os_str(), ".");
        assert!(!args.no_open);
    }

    #[test]
    fn no_open_flag_is_accepted() {
        let args = StudioArgs::try_parse_from(["kbshff-studio", "--no-open"]).expect("parse");
        assert!(args.no_open);
        assert!(!args.open);
    }
}
