use std::collections::HashMap;

use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent};

use crate::agent::agent::ToolCallInfo;

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
/// Uses priority-based compaction: the largest tool results are compacted first
/// until the trace fits within `max_chars`. The most recent `keep_recent` messages
/// are always preserved. Compacted entries are replaced with a descriptive message
/// that includes the tool name and primary parameter (e.g. file path) so the LLM
/// knows what was compacted.
pub fn compact_trace_if_needed(
    trace: &mut Vec<ChatMessage>,
    max_chars: usize,
    tool_metadata: &HashMap<String, ToolCallInfo>,
) -> bool {
    let total = estimate_trace_chars(trace);
    if total <= max_chars {
        return false;
    }

    let keep_recent = 100;
    if trace.len() <= keep_recent {
        return false;
    }

    let cutoff = trace.len() - keep_recent;

    // Collect indices of compactable tool messages (older than keep_recent) with their sizes
    let mut tool_indices: Vec<(usize, usize)> = Vec::new();
    for i in 0..cutoff {
        if let ChatMessage::Tool { content, .. } = &trace[i] {
            let size = match content {
                ChatMessageContent::Text(s) => s.len(),
                _ => 0,
            };
            // Only compact messages that are not already compacted
            if size > "[compacted]".len() {
                tool_indices.push((i, size));
            }
        }
    }

    // Sort by size descending — compact the largest entries first
    tool_indices.sort_by(|a, b| b.1.cmp(&a.1));

    let mut compacted = false;
    let mut current_total = total;

    for &(idx, _size) in &tool_indices {
        if current_total <= max_chars {
            break;
        }

        let old_size = match &trace[idx] {
            ChatMessage::Tool { content, .. } => match content {
                ChatMessageContent::Text(s) => s.len(),
                _ => 0,
            },
            _ => continue,
        };

        if let ChatMessage::Tool { tool_call_id, .. } = &trace[idx] {
            let replacement = tool_metadata
                .get(tool_call_id)
                .map(|info| {
                    match &info.primary_param {
                        Some(param) => format!("[compacted] {}({})", info.tool_name, param),
                        None => format!("[compacted] {}", info.tool_name),
                    }
                })
                .unwrap_or_else(|| "[compacted]".to_string());

            let new_size = replacement.len();
            trace[idx] = ChatMessage::Tool {
                tool_call_id: tool_call_id.clone(),
                content: ChatMessageContent::Text(replacement),
            };
            current_total = current_total - old_size + new_size;
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
        let metadata = HashMap::new();
        let result = compact_trace_if_needed(&mut trace, 10000, &metadata);
        assert!(!result);
    }

    #[test]
    fn test_compaction_replaces_old_tool_results() {
        let mut trace = Vec::new();
        let mut metadata = HashMap::new();
        for i in 0..120 {
            let id = format!("call-{}", i);
            trace.push(ChatMessage::Tool {
                tool_call_id: id.clone(),
                content: ChatMessageContent::Text("x".repeat(1000)),
            });
            metadata.insert(id, ToolCallInfo {
                tool_name: "read".to_string(),
                primary_param: Some(format!("src/file_{}.rs", i)),
            });
        }
        let result = compact_trace_if_needed(&mut trace, 5000, &metadata);
        assert!(result);

        let compacted_count = trace.iter().filter(|m| {
            matches!(m, ChatMessage::Tool { content, .. } if matches!(content, ChatMessageContent::Text(t) if t.starts_with("[compacted]")))
        }).count();
        assert!(compacted_count > 0);
    }

    #[test]
    fn test_keeps_recent_messages_intact() {
        let mut trace = Vec::new();
        let mut metadata = HashMap::new();
        for i in 0..120 {
            let id = format!("call-{}", i);
            trace.push(ChatMessage::Tool {
                tool_call_id: id.clone(),
                content: ChatMessageContent::Text("x".repeat(1000)),
            });
            metadata.insert(id, ToolCallInfo {
                tool_name: "read".to_string(),
                primary_param: Some("src/file.rs".to_string()),
            });
        }
        let _ = compact_trace_if_needed(&mut trace, 5000, &metadata);

        // Last 100 messages should still be the original content
        for i in (trace.len().saturating_sub(100))..trace.len() {
            if let ChatMessage::Tool { content, .. } = &trace[i] {
                match content {
                    ChatMessageContent::Text(t) => assert_eq!(t, &("x".repeat(1000))),
                    _ => panic!("Expected text content"),
                }
            }
        }
    }

    #[test]
    fn test_priority_compaction_largest_first() {
        let mut trace = Vec::new();
        let mut metadata = HashMap::new();

        // Small tool result
        trace.push(ChatMessage::Tool {
            tool_call_id: "small".to_string(),
            content: ChatMessageContent::Text("x".repeat(100)),
        });
        // Large tool result
        trace.push(ChatMessage::Tool {
            tool_call_id: "large".to_string(),
            content: ChatMessageContent::Text("y".repeat(5000)),
        });
        // Small tool result
        trace.push(ChatMessage::Tool {
            tool_call_id: "small2".to_string(),
            content: ChatMessageContent::Text("z".repeat(100)),
        });

        metadata.insert("small".to_string(), ToolCallInfo {
            tool_name: "read".to_string(),
            primary_param: Some("small.rs".to_string()),
        });
        metadata.insert("large".to_string(), ToolCallInfo {
            tool_name: "read".to_string(),
            primary_param: Some("large.rs".to_string()),
        });
        metadata.insert("small2".to_string(), ToolCallInfo {
            tool_name: "read".to_string(),
            primary_param: Some("small2.rs".to_string()),
        });

        // Add padding to exceed keep_recent
        for i in 0..110 {
            let id = format!("pad-{}", i);
            trace.push(ChatMessage::Tool {
                tool_call_id: id.clone(),
                content: ChatMessageContent::Text("p".repeat(200)),
            });
            metadata.insert(id, ToolCallInfo {
                tool_name: "bash".to_string(),
                primary_param: None,
            });
        }

        let result = compact_trace_if_needed(&mut trace, 10000, &metadata);
        assert!(result);

        // The large entry should be compacted (it's the biggest)
        let large_compacted = trace.iter().any(|m| {
            matches!(m, ChatMessage::Tool { content, .. } 
                if matches!(content, ChatMessageContent::Text(t) if t.contains("[compacted]")))
        });
        assert!(large_compacted);
    }
}
