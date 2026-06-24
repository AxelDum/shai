use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent};

/// Estimate the total character length of a trace by summing message content lengths.
fn estimate_trace_chars(trace: &[ChatMessage]) -> usize {
    trace
        .iter()
        .map(|msg| {
            let text = match msg {
                ChatMessage::System { content, .. }
                | ChatMessage::User { content, .. }
                | ChatMessage::Tool { content, .. } => match content {
                    ChatMessageContent::Text(s) => s.len(),
                    _ => 0,
                },
                ChatMessage::Assistant { content, .. } => content
                    .as_ref()
                    .and_then(|c| match c {
                        ChatMessageContent::Text(s) => Some(s.len()),
                        _ => None,
                    })
                    .unwrap_or(0),
                ChatMessage::Developer { content, .. } => match content {
                    ChatMessageContent::Text(s) => s.len(),
                    _ => 0,
                },
            };
            text
        })
        .sum()
}

/// Compact older tool results in the trace when it exceeds `max_chars`.
///
/// Keeps the most recent `keep_recent` messages intact. For older `ChatMessage::Tool`
/// entries, replaces the content with `[compacted]`. Returns true if any compaction
/// was performed.
pub fn compact_trace_if_needed(trace: &mut Vec<ChatMessage>, max_chars: usize) -> bool {
    let total = estimate_trace_chars(trace);
    if total <= max_chars {
        return false;
    }

    let keep_recent = 10;
    if trace.len() <= keep_recent {
        return false;
    }

    let mut compacted = false;
    let cutoff = trace.len() - keep_recent;

    for i in 0..cutoff {
        if let ChatMessage::Tool { tool_call_id, .. } = &trace[i] {
            trace[i] = ChatMessage::Tool {
                tool_call_id: tool_call_id.clone(),
                content: ChatMessageContent::Text("[compacted]".to_string()),
            };
            compacted = true;
        }
    }

    compacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_compaction_needed() {
        let mut trace = vec![ChatMessage::User {
            content: ChatMessageContent::Text("hello".to_string()),
            name: None,
        }];
        let result = compact_trace_if_needed(&mut trace, 10000);
        assert!(!result);
    }

    #[test]
    fn test_compaction_replaces_old_tool_results() {
        let mut trace = Vec::new();
        for _ in 0..20 {
            trace.push(ChatMessage::Tool {
                tool_call_id: "test".to_string(),
                content: ChatMessageContent::Text("x".repeat(1000)),
            });
        }
        let result = compact_trace_if_needed(&mut trace, 5000);
        assert!(result);

        let compacted_count = trace.iter().filter(|m| {
            matches!(m, ChatMessage::Tool { content, .. } if matches!(content, ChatMessageContent::Text(t) if t == "[compacted]"))
        }).count();
        assert!(compacted_count > 0);
    }

    #[test]
    fn test_keeps_recent_messages_intact() {
        let mut trace = Vec::new();
        for _ in 0..20 {
            trace.push(ChatMessage::Tool {
                tool_call_id: "test".to_string(),
                content: ChatMessageContent::Text("x".repeat(1000)),
            });
        }
        let _ = compact_trace_if_needed(&mut trace, 5000);

        // Last 10 messages should still be the original content
        for i in (trace.len() - 10)..trace.len() {
            if let ChatMessage::Tool { content, .. } = &trace[i] {
                match content {
                    ChatMessageContent::Text(t) => assert_eq!(t, &("x".repeat(1000))),
                    _ => panic!("Expected text content"),
                }
            }
        }
    }
}
