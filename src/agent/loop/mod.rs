//! Agent iterative LLM loop: directive parse, history pruning, and engine orchestration.

mod directive;
mod engine;
mod history;

pub use engine::{AgentEngine, AgentError, AgentRunRequest};
