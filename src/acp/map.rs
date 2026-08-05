//! Map Kuibysheff engine events / stop reasons onto ACP schema types.

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason as AcpStopReason, TextContent, ToolCall,
    ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};

use crate::agent::AgentEvent;
use crate::output::StopReason;

/// Convert a Kuibysheff [`StopReason`] into an ACP prompt stop reason.
#[must_use]
pub fn map_stop_reason(stop: StopReason, cancelled: bool) -> AcpStopReason {
    if cancelled {
        return AcpStopReason::Cancelled;
    }
    match stop {
        StopReason::GoalReached => AcpStopReason::EndTurn,
        StopReason::LimitReached => AcpStopReason::MaxTokens,
        StopReason::Error => AcpStopReason::Refusal,
    }
}

/// Convert an engine event into an ACP `session/update` payload.
#[must_use]
pub fn map_agent_event(event: AgentEvent) -> SessionUpdate {
    match event {
        AgentEvent::Thought(text) => SessionUpdate::AgentThoughtChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        )),
        AgentEvent::Message(text) => SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        )),
        AgentEvent::ToolStart {
            id,
            server,
            tool,
            arguments,
        } => {
            let title = format!("{server}.{tool}");
            let kind = tool_kind_for(&server, &tool);
            SessionUpdate::ToolCall(
                ToolCall::new(id, title)
                    .kind(kind)
                    .status(ToolCallStatus::InProgress)
                    .raw_input(arguments),
            )
        }
        AgentEvent::ToolFinish { id, ok, output } => {
            let status = if ok {
                ToolCallStatus::Completed
            } else {
                ToolCallStatus::Failed
            };
            let content = ToolCallContent::from(json_preview(&output));
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                id,
                ToolCallUpdateFields::new()
                    .status(status)
                    .content(vec![content])
                    .raw_output(output),
            ))
        }
    }
}

fn tool_kind_for(server: &str, tool: &str) -> ToolKind {
    match (server, tool) {
        ("home", "read" | "list") => ToolKind::Read,
        ("home", "write") => ToolKind::Edit,
        ("home", "run") => ToolKind::Execute,
        ("local_tools", "search_docs") => ToolKind::Search,
        ("local_tools", "read_file") => ToolKind::Read,
        _ => ToolKind::Other,
    }
}

fn json_preview(value: &serde_json::Value) -> String {
    let raw = value.to_string();
    const MAX: usize = 4_000;
    if raw.len() <= MAX {
        raw
    } else {
        format!("{}…", &raw[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_goal_to_end_turn() {
        assert_eq!(
            map_stop_reason(StopReason::GoalReached, false),
            AcpStopReason::EndTurn
        );
    }

    #[test]
    fn cancel_overrides_stop_reason() {
        assert_eq!(
            map_stop_reason(StopReason::GoalReached, true),
            AcpStopReason::Cancelled
        );
    }

    #[test]
    fn maps_thought_chunk() {
        let update = map_agent_event(AgentEvent::Thought("plan".into()));
        assert!(matches!(update, SessionUpdate::AgentThoughtChunk(_)));
    }
}
