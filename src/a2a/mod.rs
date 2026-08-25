//! Agent-to-Agent (A2A) 1.0 HTTP server for peer agents.
//!
//! Transport, task store, and JSON-RPC/REST routers come from the official
//! `a2a-server-lf` SDK. This module wires [`crate::app::run_agent_prompt`] as an
//! [`a2a_server::AgentExecutor`] and publishes an Agent Card from the profile.

mod auth;
mod card;
mod executor;
mod server;

pub use server::run_a2a_server;
