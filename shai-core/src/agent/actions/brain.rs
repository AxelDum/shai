use chrono::Utc;
use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent};
use tracing::info;
use tokio_util::sync::CancellationToken;
use crate::agent::{AgentCore, AgentError, AgentEvent, InternalAgentEvent, InternalAgentState, ThinkerContext, ThinkerDecision, ThinkerFlowControl};

impl AgentCore {
    /// Launch a brain task to decide next step
    pub async fn spawn_next_step(&mut self) {         
        let cancellation_token = CancellationToken::new();
        let cancel_token_clone = cancellation_token.clone();
        let trace = self.trace.clone();
        let tx_clone = self.internal_tx.clone();
        let available_tools = self.available_tools.clone();
        let method = self.method.clone();
        let max_trace_chars = self.compaction_config.max_trace_chars;
        let context = ThinkerContext {
            trace,
            available_tools,
            method,
            max_trace_chars,
        };
        let brain = self.brain.clone();
        
        //////////////////////// TOKIO SPAWN
        tokio::spawn(async move {
            tokio::select! {
                result = async {
                    brain.write().await.next_step(context).await
                } => {
                    let _ = tx_clone.send(InternalAgentEvent::BrainResult {
                        result
                    });
                }
                _ = cancel_token_clone.cancelled() => {
                    // Brain thinking was cancelled, no need to send result
                }
            }
        });
        //////////////////////// TOKIO SPAWN
        
        self.set_state(InternalAgentState::Processing { 
            task_name: "next_step".to_string(), 
            tools_exec_at: Utc::now(), 
            cancellation_token
        }).await;
    }


    /// Process a brain task result
    pub async fn process_next_step(&mut self, result: Result<ThinkerDecision, AgentError>) -> Result<(), AgentError> {
        let ThinkerDecision{message, flow, token_usage} = self.handle_brain_error(result).await?;
        let ChatMessage::Assistant { content, reasoning_content, tool_calls, .. } = message.clone() else {
            return self.handle_brain_error::<ThinkerDecision>(
                Err(AgentError::InvalidResponse(format!("ChatMessage::Assistant expected, but got {:?} instead", message)))).await.map(|_| ()
            );
        };
    
        // Add the message to trace
        info!(target: "agent::think", reasoning_content = ?reasoning_content, content = ?content);
        let trace = self.trace.clone();
        trace.write().await.push(message.clone());
        
        // Emit event to external consumers
        let _ = self.emit_event(AgentEvent::BrainResult {
            timestamp: Utc::now(),
            thought: Ok(message.clone())
        }).await;

        // Emit token usage event if available
        if let Some((input_tokens, output_tokens, cached_tokens)) = token_usage {
            let _ = self.emit_event(AgentEvent::TokenUsage {
                input_tokens,
                output_tokens,
                cached_tokens
            }).await;
        }
    
        // run tool call if any
        let tool_calls_from_brain = tool_calls.unwrap_or(vec![]);
        if !tool_calls_from_brain.is_empty() {
            // Check max tool calls per turn limit
            if let Some(max_tool_calls) = self.compaction_config.max_tool_calls_per_turn {
                let mut count = self.tool_call_count.write().await;
                if *count >= max_tool_calls {
                    // Drop the guard before calling set_state
                    drop(count);
                    // Inject a wrap-up message for each tool call to satisfy the LLM's tool_call_id requirements
                    let wrap_up = format!(
                        "You have reached the maximum number of tool calls ({}) for this turn. Please summarize what you've accomplished and provide your final answer.",
                        max_tool_calls
                    );
                    for tc in &tool_calls_from_brain {
                        self.trace.write().await.push(ChatMessage::Tool {
                            tool_call_id: tc.id.clone(),
                            content: ChatMessageContent::Text(wrap_up.clone()),
                        });
                    }
                    self.set_state(InternalAgentState::Running).await;
                    return Ok(());
                }
                *count += tool_calls_from_brain.len();
            }
            self.spawn_tools(tool_calls_from_brain).await;
            return Ok(())
        }

        // no tool call, thus we rely on flow control
        match flow {
            ThinkerFlowControl::AgentContinue => {
                self.set_state(InternalAgentState::Running).await;
            }
            ThinkerFlowControl::AgentPause => {
                self.set_state(InternalAgentState::Paused).await;
            }
        }
        Ok(())
    }

    // Helper method that emits error events before returning the error
    async fn handle_brain_error<T>(&mut self, result: Result<T, AgentError>) -> Result<T, AgentError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.set_state(InternalAgentState::Paused).await;
                let _ = self.emit_event(AgentEvent::BrainResult { 
                    timestamp: Utc::now(),
                    thought: Err(error.clone())
                }).await;
                Err(error)
            }
        }
    }
}