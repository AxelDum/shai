use std::sync::Arc;

use crate::agent::{
    AgentCore, AgentEvent, ClaimManager, InternalAgentEvent, InternalAgentState, PermissionRequest,
    PermissionResponse,
};
use crate::runners::compacter::compact_tool_result;
use crate::tools::{AnyTool, ToolCall, ToolCapability, ToolResult};
use chrono::{TimeDelta, Utc};
use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent, ToolCall as LlmToolCall};
use serde_json::from_str;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::info;
use uuid::Uuid;

impl AgentCore {
    /// Spawn a cancellable coroutine that runs all tool call in parrallel and waits for them to finish
    pub async fn spawn_tools(&mut self, tool_calls: Vec<LlmToolCall>) {
        let cancellation_token = CancellationToken::new();
        let cancel_clone = cancellation_token.clone();
        let internal_tx = self.internal_tx.clone();

        // Clone all needed data from self before spawning
        let public_event_tx = self.socket.tx_event.clone();
        let available_tools = self.available_tools.clone();
        let claims = self.permissions.clone();
        let trace = self.trace.clone();
        let compaction_config = self.compaction_config.clone();
        let working_dir = self.working_dir.clone();
        let command_cache = self.command_cache.clone();
        let read_cache = self.read_cache.clone();
        let max_cached_commands = self.compaction_config.max_cached_commands;
        let max_cached_reads = self.compaction_config.max_cached_reads;

        // Spawn a task to wait for all tool executions
        let mut join_handles = Vec::new();

        // Spawn all tool executions
        for tc in tool_calls {
            let handle = Self::spawn_tool_static(
                tc,
                cancel_clone.clone(),
                public_event_tx.clone(),
                available_tools.clone(),
                claims.clone(),
                internal_tx.clone(),
                trace.clone(),
                compaction_config.clone(),
                working_dir.clone(),
                command_cache.clone(),
                max_cached_commands,
                self.todo_storage.clone(),
                read_cache.clone(),
                max_cached_reads,
            );
            join_handles.push(handle);
        }

        // Wait for all tools to complete or be cancelled
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    // Tools were cancelled, no need to send completion event
                }
                any_denied = async {
                    // wait for all tools completion and collect denial status
                    let mut result = false;
                    for handle in join_handles {
                        if let Ok(was_denied) = handle.await {
                            result = result || was_denied;
                        }
                    }
                    result
                } => {
                    // All tools completed, move to Running state
                    let _ = internal_tx.send(InternalAgentEvent::ToolsCompleted { any_denied });
                }
            }
        });

        // Set state to Processing with cancellation token
        self.set_state(InternalAgentState::Processing {
            task_name: "tools".to_string(),
            tools_exec_at: Utc::now(),
            cancellation_token,
        })
        .await;
    }

    /// Spawn a cancellable coroutine that runs a single tool call
    /// coordinating the appropriate tool specific event (start/completed)
    fn spawn_tool_static(
        tc: LlmToolCall,
        cancel_token: CancellationToken,
        public_event_tx: Option<broadcast::Sender<AgentEvent>>,
        available_tools: Vec<Arc<dyn AnyTool>>,
        claims: Arc<RwLock<ClaimManager>>,
        internal_tx: broadcast::Sender<InternalAgentEvent>,
        trace: Arc<RwLock<Vec<ChatMessage>>>,
        compaction_config: crate::config::agent::CompactionConfig,
        working_dir: Option<String>,
        command_cache: Arc<RwLock<Vec<(String, String)>>>,
        max_cached_commands: usize,
        todo_storage: Arc<crate::tools::todo::TodoStorage>,
        read_cache: Arc<RwLock<Vec<(String, String)>>>,
        max_cached_reads: usize,
    ) -> tokio::task::JoinHandle<bool> {
        tokio::spawn(async move {
            let tc_for_error = tc.clone();
            match Self::tool_exist(available_tools, tc) {
                // tool does not exist, we fail immediately
                Err(tool_result) => {
                    if let Some(tx) = public_event_tx.clone() {
                        let _ = tx.send(AgentEvent::ToolCallCompleted {
                            duration: TimeDelta::zero(),
                            call: ToolCall {
                                tool_call_id: tc_for_error.id.clone(),
                                tool_name: tc_for_error.function.name.clone(),
                                parameters: serde_json::Value::Null,
                            },
                            result: tool_result,
                            original_bytes: 0,
                            compacted_bytes: 0,
                        });
                    }
                    false
                }

                Ok((tool, mut call)) => {
                    // Inject agent's working_dir as default for bash tool if not specified
                    if call.tool_name == "bash" {
                        if let Some(dir) = &working_dir {
                            if let Some(obj) = call.parameters.as_object_mut() {
                                obj.entry("working_dir")
                                    .or_insert(serde_json::Value::String(dir.clone()));
                            }
                        }
                    }

                    let start = Utc::now();

                    // Emit tool call started event
                    if let Some(tx) = public_event_tx.clone() {
                        let _ = tx.send(AgentEvent::ToolCallStarted {
                            timestamp: start.clone(),
                            call: call.clone(),
                        });
                    }

                    // Normalize command for cache lookup (bash only)
                    let normalized_command = if call.tool_name == "bash" {
                        call.parameters
                            .get("command")
                            .and_then(|v| v.as_str())
                            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                    } else {
                        None
                    };

                    // Check command cache
                    let cached_output = if let Some(ref cmd) = normalized_command {
                        let cache = command_cache.read().await;
                        cache
                            .iter()
                            .rev()
                            .find(|(c, _)| c == cmd)
                            .map(|(_, output)| output.clone())
                    } else {
                        None
                    };

                    // Check read cache for read tool calls
                    let read_cache_key = if call.tool_name == "read" {
                        // Build cache key from all file specs in the files array
                        call.parameters
                            .get("files")
                            .and_then(|v| v.as_array())
                            .map(|files| {
                                files
                                    .iter()
                                    .map(|f| {
                                        let path =
                                            f.get("path").and_then(|v| v.as_str()).unwrap_or("");
                                        let offset = f
                                            .get("offset")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let limit = f
                                            .get("limit")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let outline = f.get("outline").and_then(|v| v.as_bool()).unwrap_or(false);
                                        format!("{}:{}:{}:{}", path, offset, limit, outline)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("|")
                            })
                    } else {
                        None
                    };

                    let cached_read = if let Some(ref _key) = read_cache_key {
                        let cache = read_cache.read().await;
                        cache
                            .iter()
                            .rev()
                            .find(|(c, _)| c == _key.as_str())
                            .map(|(_, output)| output.clone())
                    } else {
                        None
                    };

                    // Execute or use cached result
                    let result: ToolResult = if let Some(cached) = cached_output {
                        ToolResult::success(format!(
                            "[cached] You just ran this command. The result is the same.\n\n{}",
                            cached
                        ))
                    } else if let Some(ref cached) = cached_read {
                        ToolResult::success(format!(
                            "[cached] This file was read recently and hasn't changed.\n\n{}",
                            cached
                        ))
                    } else {
                        // execute tool
                        let tool_handle = Self::spawn_tool_exec(
                            tool,
                            call.clone(),
                            cancel_token.clone(),
                            claims,
                            public_event_tx.clone(),
                            internal_tx.subscribe(),
                        );

                        // wait for result (or for cancellation)
                        let exec_result = tokio::select! {
                            join_result = tool_handle => {
                                match join_result {
                                    Ok(tool_result) => tool_result,
                                    Err(join_error) => {
                                        debug!(target: "agent::tool_completed", "tool execution task failed: {}", join_error);
                                        ToolResult::error(format!("tool execution task failed: {}", join_error))
                                    }
                                }
                            },
                            _ = cancel_token.cancelled() => {
                                debug!(target: "agent::tool_completed", "cancelled by user");
                                ToolResult::error("tool call was cancelled by the user".to_string())
                            }
                        };

                        // Cache the result if it was a bash command
                        if let Some(ref cmd) = normalized_command {
                            let compacted = compact_tool_result(
                                &call.tool_name,
                                exec_result.metadata(),
                                &exec_result.to_string(),
                                &compaction_config,
                            );
                            let mut cache = command_cache.write().await;
                            cache.push((cmd.clone(), compacted));
                            if cache.len() > max_cached_commands {
                                cache.remove(0);
                            }
                        }

                        // Cache the result if it was a read command and was successful
                        if let Some(ref key) = read_cache_key {
                            if exec_result.is_success() {
                                let compacted = compact_tool_result(
                                    &call.tool_name,
                                    exec_result.metadata(),
                                    &exec_result.to_string(),
                                    &compaction_config,
                                );
                                let mut cache = read_cache.write().await;
                                cache.push((key.clone(), compacted));
                                if cache.len() > max_cached_reads {
                                    cache.remove(0);
                                }
                            }
                        }

                        // Invalidate read cache entries for edited files
                        if call.tool_name == "edit" || call.tool_name == "write" {
                            let edited_paths: Vec<String> = if call.tool_name == "edit" {
                                // edit tool uses files: [{file_path, edits: [...]}]
                                call.parameters
                                    .get("files")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|f| {
                                                f.get("file_path")
                                                    .and_then(|p| p.as_str())
                                                    .map(String::from)
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default()
                            } else {
                                // write tool uses files: [{path, content}]
                                call.parameters
                                    .get("files")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|f| {
                                                f.get("path")
                                                    .and_then(|p| p.as_str())
                                                    .map(String::from)
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default()
                            };
                            if !edited_paths.is_empty() {
                                let mut cache = read_cache.write().await;
                                cache.retain(|(key, _)| {
                                    !edited_paths
                                        .iter()
                                        .any(|p| key.starts_with(&format!("{}:", p)) || key == p)
                                });
                            }
                        }

                        exec_result
                    };

                    // Compact and push result to trace
                    let raw_output = result.to_string();
                    let compacted_output = compact_tool_result(
                        &call.tool_name,
                        result.metadata(),
                        &raw_output,
                        &compaction_config,
                    );
                    let _ = {
                        trace.write().await.push(ChatMessage::Tool {
                            tool_call_id: call.tool_call_id.clone(),
                            content: ChatMessageContent::Text(compacted_output.clone()),
                        });
                    };

                    // Emit tool call finish event
                    let tool_was_denied = result.is_denied();
                    info!(target: "agent::tool_completed", call = ?tc_for_error.function.name.clone(), result = ?result);
                    if let Some(tx) = public_event_tx.clone() {
                        let _ = tx.send(AgentEvent::ToolCallCompleted {
                            duration: Utc::now() - start,
                            call: call,
                            result,
                            original_bytes: raw_output.len(),
                            compacted_bytes: compacted_output.len(),
                        });
                    }

                    // Emit TodoUpdated event after todo_write executes
                    if tc_for_error.function.name == "todo_write" {
                        let todos = todo_storage.get_all().await;
                        if let Some(tx) = public_event_tx.clone() {
                            let _ = tx.send(AgentEvent::TodoUpdated { todos });
                        }
                    }

                    tool_was_denied
                }
            }
        })
    }

    /// execute a single tool call
    /// checking for permission, requesting it, executing the tool
    fn spawn_tool_exec(
        tool: Arc<dyn AnyTool>,
        call: ToolCall,
        cancel_token: CancellationToken,
        claims: Arc<RwLock<ClaimManager>>,
        public_event_tx: Option<broadcast::Sender<AgentEvent>>,
        mut internal_rx: broadcast::Receiver<InternalAgentEvent>,
    ) -> JoinHandle<ToolResult> {
        tokio::spawn(async move {
            // check permission, we allow all Read Tool
            let can_run = tool.capabilities().is_empty()
                || tool.capabilities() == &[ToolCapability::Read]
                || claims
                    .read()
                    .await
                    .is_permitted(&tool.name(), &call.parameters);

            // request permission if needed (|| is short-circuiting, so won't call if can_run is true)
            let can_run = can_run
                || match Self::request_permission_if_needed(
                    &call,
                    &tool,
                    &public_event_tx,
                    &mut internal_rx,
                    &cancel_token,
                )
                .await
                {
                    Ok(permission_granted) => permission_granted,
                    Err(preview_error) => return preview_error, // Return preview error immediately
                };

            if !can_run {
                let claims_guard = claims.read().await;
                if claims_guard.is_plan_mode() {
                    return ToolResult::error(
                        "Tool execution is disabled in PLAN mode. Do not attempt to write, edit, or execute commands. Instead, describe the changes you would make as a detailed plan.".to_string(),
                    );
                }
                return ToolResult::denied();
            }

            // Execute tool with cancellation support
            tokio::select! {
                result = tool.execute_json(call.parameters.clone(), Some(cancel_token.clone())) => result,
                _ = cancel_token.cancelled() => {
                    ToolResult::error("tool call was cancelled by the user".to_string())
                }
            }
        })
    }

    /// send a permission request (if necessary) and wait for the answer
    /// Returns Ok(true) if permission granted, Ok(false) if denied, Err(ToolResult) if preview failed
    async fn request_permission_if_needed(
        call: &ToolCall,
        tool: &Arc<dyn AnyTool>,
        public_event_tx: &Option<broadcast::Sender<AgentEvent>>,
        internal_rx: &mut broadcast::Receiver<InternalAgentEvent>,
        cancel_token: &CancellationToken,
    ) -> Result<bool, ToolResult> {
        // Session is not interactive so we cannot ask for permission
        let Some(tx) = public_event_tx.as_ref() else {
            return Ok(false);
        };

        // Try to get preview from tool
        let preview = tool.execute_preview_json(call.parameters.clone()).await;

        // If preview returned an error, return that error immediately
        if let Some(error_result) = &preview {
            if let ToolResult::Error { .. } = error_result {
                return Err(error_result.clone());
            }
        }

        // Send permission request
        let req_id = Uuid::new_v4().to_string();
        let _ = tx.send(AgentEvent::PermissionRequired {
            request_id: req_id.clone(),
            request: PermissionRequest {
                tool_name: call.tool_name.clone(),
                operation: "do you want to run this tool?".to_string(),
                call: call.clone(),
                preview,
            },
        });

        // Wait for permission response
        loop {
            tokio::select! {
                recv_result = internal_rx.recv() => {
                    match recv_result {
                        Ok(InternalAgentEvent::PermissionResponseReceived { request_id, response }) if request_id == req_id => {
                            return Ok(matches!(response, PermissionResponse::Allow | PermissionResponse::AllowAlways));
                        }
                        Ok(_) => continue,
                        Err(_) => return Ok(false), // Channel closed
                    }
                }
                _ = cancel_token.cancelled() => {
                    return Ok(false); // Cancelled during permission wait
                }
            }
        }
    }

    // utility method
    fn tool_exist(
        tools: Vec<Arc<dyn AnyTool>>,
        tc: LlmToolCall,
    ) -> Result<(Arc<dyn AnyTool>, ToolCall), ToolResult> {
        from_str(&tc.function.arguments)
            .map_err(|_e| ToolResult::error("failed to parse tool parameters".to_string()))
            .and_then(|params| {
                let tool_call = ToolCall {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.function.name.clone(),
                    parameters: params,
                };

                // Find the tool
                tools
                    .iter()
                    .find(|t| t.name() == tool_call.tool_name)
                    .cloned()
                    .ok_or_else(|| {
                        ToolResult::error(format!("tool not found: {}", tool_call.tool_name))
                    })
                    .map(|tool| (tool, tool_call))
            })
    }
}
