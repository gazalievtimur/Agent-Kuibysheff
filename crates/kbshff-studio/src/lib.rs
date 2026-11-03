//! Local operator studio. Every agent operation is a `kbshff` subprocess.

mod api;
mod cli;
mod interrupt;
mod jobs;
mod kbshff;
mod server;

pub use api::{router, test_router as api_test_router};
pub use cli::StudioArgs;
pub use jobs::{parse_progress_line, JobManager};
pub use kbshff::{resolve_kbshff, Kbshff};
pub use server::{run_studio, StudioState};
