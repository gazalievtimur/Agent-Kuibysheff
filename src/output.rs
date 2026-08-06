use serde::Serialize;

use crate::billing::RunCostReport;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    GoalReached,
    LimitReached,
    Error,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UsageReport {
    pub iterations: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub elapsed_ms: u128,
    pub cost: RunCostReport,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LogReport {
    pub ai_log: Option<String>,
    pub mcp_log: Option<String>,
    pub system_log: Option<String>,
    pub chat_log: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunOutput {
    pub run_id: String,
    pub result: String,
    pub usage: UsageReport,
    pub stop_reason: StopReason,
    pub logs: LogReport,
}

impl RunOutput {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            run_id: "setup-error".to_string(),
            result: message.into(),
            usage: UsageReport::default(),
            stop_reason: StopReason::Error,
            logs: LogReport::default(),
        }
    }
}
