use crate::config::ProviderHistoryConfig;
use crate::provider::ChatMessage;

/// Prefix length: system prompt + initial user goal (not configurable).
pub(crate) const HISTORY_PREFIX_LEN: usize = 2;

pub(crate) fn push_message(
    messages: &mut Vec<ChatMessage>,
    full_history: &mut Vec<ChatMessage>,
    message: ChatMessage,
    history: &ProviderHistoryConfig,
) {
    full_history.push(message.clone());
    prune_message_history(full_history, history);
    messages.push(message);
}

/// Keeps the system prompt and initial user message, dropping oldest middle turns
/// by message count and by total character budget from `history`.
pub(crate) fn prune_message_history(
    messages: &mut Vec<ChatMessage>,
    history: &ProviderHistoryConfig,
) {
    prune_by_message_count(messages, history);
    prune_by_char_budget(messages, history);
}

fn prune_by_message_count(messages: &mut Vec<ChatMessage>, history: &ProviderHistoryConfig) {
    let max_tail = history.max_tail_messages;
    let max_total = HISTORY_PREFIX_LEN.saturating_add(max_tail);
    if messages.len() <= max_total {
        return;
    }
    let tail_start = messages.len() - max_tail;
    if tail_start <= HISTORY_PREFIX_LEN {
        return;
    }
    let tail: Vec<ChatMessage> = messages.drain(tail_start..).collect();
    messages.truncate(HISTORY_PREFIX_LEN);
    messages.extend(tail);
}

fn prune_by_char_budget(messages: &mut Vec<ChatMessage>, history: &ProviderHistoryConfig) {
    while messages.len() > HISTORY_PREFIX_LEN && history_char_len(messages) > history.max_chars {
        if messages.len() == HISTORY_PREFIX_LEN + 1 {
            let prefix_chars = history_char_len(&messages[..HISTORY_PREFIX_LEN]);
            let budget_for_tail = history.max_chars.saturating_sub(prefix_chars);
            if budget_for_tail == 0 {
                messages.remove(HISTORY_PREFIX_LEN);
                continue;
            }

            let tail_idx = HISTORY_PREFIX_LEN;
            let tail = messages[tail_idx].content.as_ref();
            let truncated_tail = truncate_for_budget(tail, budget_for_tail);
            if truncated_tail.chars().count() < tail.chars().count() {
                messages[tail_idx].content = truncated_tail.into();
                continue;
            }
        }
        // Drop the oldest non-prefix turn; prefer retaining recent context.
        messages.remove(HISTORY_PREFIX_LEN);
    }
}

fn truncate_for_budget(content: &str, max_chars: usize) -> String {
    const MARKER: &str = "\n...[truncated for history budget]";
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let marker_chars = MARKER.chars().count();
    if max_chars <= marker_chars {
        return content.chars().take(max_chars).collect();
    }

    let head_chars = max_chars - marker_chars;
    let mut out = String::with_capacity(max_chars.saturating_add(8));
    out.extend(content.chars().take(head_chars));
    out.push_str(MARKER);
    out
}

pub(crate) fn history_char_len(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|message| message.content.chars().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ChatRole;

    fn test_history() -> ProviderHistoryConfig {
        ProviderHistoryConfig::default()
    }

    #[test]
    fn push_message_prunes_full_history_to_same_budget_as_messages() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        let mut full_history = messages.clone();

        for i in 0..40 {
            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::Assistant, format!("assistant-{i}")),
                &history,
            );
            push_message(
                &mut messages,
                &mut full_history,
                ChatMessage::new(ChatRole::User, format!("user-{i}")),
                &history,
            );
            prune_message_history(&mut messages, &history);
        }

        assert_eq!(full_history.len(), messages.len());
        assert!(full_history.len() <= HISTORY_PREFIX_LEN + history.max_tail_messages);
        assert_eq!(full_history[0].content.as_ref(), "system");
        assert_eq!(full_history[1].content.as_ref(), "goal");
        assert_eq!(full_history.last().unwrap().content.as_ref(), "user-39");
    }

    #[test]
    fn full_history_respects_char_budget() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        let mut full_history = messages.clone();

        push_message(
            &mut messages,
            &mut full_history,
            ChatMessage::new(ChatRole::Assistant, "a".repeat(80_000)),
            &history,
        );
        push_message(
            &mut messages,
            &mut full_history,
            ChatMessage::new(ChatRole::User, "b".repeat(80_000)),
            &history,
        );
        push_message(
            &mut messages,
            &mut full_history,
            ChatMessage::new(ChatRole::Assistant, "c".repeat(80_000)),
            &history,
        );

        assert!(history_char_len(&full_history) <= history.max_chars);
        assert_eq!(full_history[0].content.as_ref(), "system");
        assert_eq!(full_history[1].content.as_ref(), "goal");
        assert_eq!(
            full_history.last().unwrap().content.as_ref(),
            &"c".repeat(80_000)
        );
    }

    #[test]
    fn prune_message_history_keeps_prefix_and_tail() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        for i in 0..40 {
            messages.push(ChatMessage::new(
                ChatRole::Assistant,
                format!("assistant-{i}"),
            ));
            messages.push(ChatMessage::new(ChatRole::User, format!("user-{i}")));
        }

        prune_message_history(&mut messages, &history);

        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        assert!(messages.len() <= HISTORY_PREFIX_LEN + history.max_tail_messages);
        assert_eq!(messages.last().unwrap().content.as_ref(), "user-39");
    }

    #[test]
    fn prune_message_history_enforces_char_budget() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        messages.push(ChatMessage::new(ChatRole::Assistant, "a".repeat(80_000)));
        messages.push(ChatMessage::new(ChatRole::User, "b".repeat(80_000)));
        messages.push(ChatMessage::new(ChatRole::Assistant, "c".repeat(80_000)));

        prune_message_history(&mut messages, &history);

        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        assert!(history_char_len(&messages) <= history.max_chars);
        assert_eq!(messages.last().unwrap().content.chars().count(), 80_000);
        assert_eq!(
            messages.last().unwrap().content.as_ref(),
            &"c".repeat(80_000)
        );
        // Oldest oversized middle turn is dropped first.
        assert!(!messages
            .iter()
            .any(|message| message.content.as_ref() == "a".repeat(80_000)));
    }

    #[test]
    fn prune_message_history_handles_single_oversized_tool_result() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
            ChatMessage::new(ChatRole::User, "t".repeat(100_000)),
            ChatMessage::new(ChatRole::Assistant, "ok"),
        ];

        prune_message_history(&mut messages, &history);

        assert!(history_char_len(&messages) <= history.max_chars);
        assert_eq!(messages[0].content.as_ref(), "system");
        assert_eq!(messages[1].content.as_ref(), "goal");
        // 100k fits under the 200k budget with prefix retained.
        assert!(messages
            .iter()
            .any(|message| message.content.chars().count() == 100_000));
    }

    #[test]
    fn prune_message_history_truncates_single_tail_when_over_budget() {
        let history = test_history();
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
            ChatMessage::new(ChatRole::User, "t".repeat(300_000)),
        ];

        prune_message_history(&mut messages, &history);

        assert_eq!(messages.len(), HISTORY_PREFIX_LEN + 1);
        assert!(history_char_len(&messages) <= history.max_chars);
        assert!(messages[HISTORY_PREFIX_LEN]
            .content
            .contains("...[truncated for history budget]"));
    }

    #[test]
    fn prune_message_history_respects_larger_configured_window() {
        let history = ProviderHistoryConfig {
            max_tail_messages: 80,
            max_chars: 500_000,
        };
        let mut messages = vec![
            ChatMessage::new(ChatRole::System, "system"),
            ChatMessage::new(ChatRole::User, "goal"),
        ];
        for i in 0..40 {
            messages.push(ChatMessage::new(
                ChatRole::Assistant,
                format!("assistant-{i}"),
            ));
            messages.push(ChatMessage::new(ChatRole::User, format!("user-{i}")));
        }

        let before_len = messages.len();
        prune_message_history(&mut messages, &history);

        assert_eq!(messages.len(), before_len);
        assert!(messages
            .iter()
            .any(|message| message.content.as_ref() == "assistant-0"));
        assert_eq!(messages.last().unwrap().content.as_ref(), "user-39");
    }
}
