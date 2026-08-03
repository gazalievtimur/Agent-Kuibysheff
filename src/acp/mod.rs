//! Agent Client Protocol (ACP) stdio server for IDE hosts (VS Code, etc.).

mod map;
mod server;

pub use server::run_acp_server;
