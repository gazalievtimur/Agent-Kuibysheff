//! Bind the loopback studio and optionally open a browser.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::cli::StudioArgs;
use crate::jobs::JobManager;
use crate::kbshff::resolve_kbshff;

use super::api;

/// Shared state for one studio process.
#[derive(Clone)]
pub struct StudioState {
    pub project_root: PathBuf,
    pub kbshff: PathBuf,
    pub jobs: JobManager,
}

/// Serve until Ctrl+C.
///
/// # Errors
///
/// Returns when the bind address is not loopback, bind fails, or `kbshff` cannot be resolved.
pub async fn run_studio(args: StudioArgs) -> Result<()> {
    let bind: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid --bind address `{}`", args.bind))?;
    if !bind.ip().is_loopback() {
        bail!(
            "studio may only bind to loopback addresses (got `{}`)",
            bind.ip()
        );
    }
    let project_root = args
        .project_root
        .canonicalize()
        .with_context(|| format!("resolve --project-root `{}`", args.project_root.display()))
        .unwrap_or(args.project_root.clone());
    let kbshff = resolve_kbshff(args.kbshff.as_deref()).map_err(|err| anyhow::anyhow!(err))?;
    let state = StudioState {
        project_root: project_root.clone(),
        kbshff,
        jobs: JobManager::new(),
    };
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind studio listener on {bind}"))?;
    let url = format!("http://{bind}/");
    eprintln!("Kuibysheff studio listening at {url}");
    eprintln!("project-root: {}", project_root.display());
    if args.open || !args.no_open {
        if let Err(err) = open_browser(&url) {
            eprintln!("warning: failed to open browser: {err:#}");
        }
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("studio HTTP server exited with error")?;
    Ok(())
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .context("spawn cmd to open browser")?;
        if !status.success() {
            bail!("cmd start exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open")
            .arg(url)
            .status()
            .context("spawn xdg-open")?;
        if !status.success() {
            bail!("xdg-open exited with {status}");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = url;
        bail!("--open is not supported on this platform")
    }
}
