use std::{collections::HashMap, io, time::Duration};
use shai_llm::ToolCallMethod;

use crate::tui::App;
use super::theme::Theme;

impl App<'_> {
    pub(crate) fn list_command() -> HashMap<(String, String),Vec<String>> {
        HashMap::from([
            (("/exit".to_string(),"exit from the tui".to_string()), Vec::<String>::new()),
            (("/auth".to_string(),"select a provider".to_string()), Vec::<String>::new()),
            (("/tc".to_string(),"set the tool call method: [fc | fc2 | so]".to_string()), vec!["method".to_string()]),
            (("/tokens".to_string(),"display token usage (input/output)".to_string()), Vec::<String>::new()),
            (("/theme".to_string(),"set theme: [dark | light | toggle]".to_string()), vec!["mode".to_string()]),
            (("/restore".to_string(),"restore a previous session".to_string()), Vec::<String>::new()),
        ])
    }

    pub(crate) async fn handle_app_command(&mut self, command: &str) -> io::Result<()> {
        let mut parts = command.split_whitespace();
        let cmd = parts.next().unwrap();
        let args: Vec<&str> = parts.collect();

        match cmd {
            "/exit" => {
                self.exit = true;
            }
            "/tc" => {
                if let Some(ref agent) = self.agent {
                    match args.into_iter().next() {
                        Some("auto") => {
                            if let Ok(method) = agent.controller.set_method(Some(ToolCallMethod::Auto)).await {
                                self.input.alert_msg("llm will now try all method for tool calls", Duration::from_secs(3));
                                self.input.set_tool_call_method(method);
                            }
                        }
                        Some("fc") => {
                            if let Ok(method) = agent.controller.set_method(Some(ToolCallMethod::FunctionCall)).await {
                                self.input.alert_msg("llm will now use function calling api for tool calls", Duration::from_secs(3));
                                self.input.set_tool_call_method(method);
                            }
                        }
                        Some("fc2") => {
                            if let Ok(method) = agent.controller.set_method(Some(ToolCallMethod::FunctionCallRequired)).await {
                                self.input.alert_msg("llm will now use function calling in required mode for tool calls", Duration::from_secs(3));
                                self.input.set_tool_call_method(method);
                            }
                        }
                        Some("so") => {
                            if let Ok(method) = agent.controller.set_method(Some(ToolCallMethod::StructuredOutput)).await {
                                self.input.alert_msg("llm will now use structured output for tool calls", Duration::from_secs(3));
                                self.input.set_tool_call_method(method);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "/tokens" => {
                let msg = format!(
                    "Token Usage - Input: {}, Output: {}, Total: {}",
                    self.total_input_tokens,
                    self.total_output_tokens,
                    self.total_input_tokens + self.total_output_tokens
                );
                self.input.alert_msg(&msg, Duration::from_secs(5));
            }
            "/theme" => {
                match args.into_iter().next() {
                    Some("dark") => {
                        self.theme = Theme::Dark;
                        let new_palette = self.theme.palette();
                        self.input.set_palette(new_palette);
                        self.input.alert_msg("Theme set to dark", Duration::from_secs(2));
                    }
                    Some("light") => {
                        self.theme = Theme::Light;
                        let new_palette = self.theme.palette();
                        self.input.set_palette(new_palette);
                        self.input.alert_msg("Theme set to light", Duration::from_secs(2));
                    }
                    Some("toggle") => {
                        self.theme.toggle();
                        let new_palette = self.theme.palette();
                        self.input.set_palette(new_palette);
                        let theme_name = match self.theme {
                            Theme::Dark => "dark",
                            Theme::Light => "light",
                        };
                        self.input.alert_msg(&format!("Theme toggled to {}", theme_name), Duration::from_secs(2));
                    }
                    _ => {
                        self.input.alert_msg("Usage: /theme [dark|light|toggle]", Duration::from_secs(3));
                    }
                }
            }
            "/restore" => {
                let sessions = shai_core::session::SessionPersist::list_sessions();
                match sessions {
                    Ok(sessions) if !sessions.is_empty() => {
                        if let Some(arg) = args.into_iter().next() {
                            // Try to match by index (1-based) or by session_id prefix
                            let selected = arg.parse::<usize>()
                                .ok()
                                .and_then(|idx| {
                                    if idx > 0 && idx <= sessions.len() {
                                        Some(&sessions[idx - 1])
                                    } else {
                                        None
                                    }
                                })
                                .or_else(|| {
                                    // Match by session_id prefix
                                    sessions.iter().find(|s| s.session_id.starts_with(arg))
                                });

                            if let Some(session) = selected {
                                self.input.alert_msg(&format!("Restoring session {}...", &session.session_id[..8]), Duration::from_secs(2));

                                // Drop existing agent
                                if let Some(agent) = self.agent.take() {
                                    let _ = agent.controller.terminate().await;
                                }

                                // Update session_id and start new agent with restored trace
                                self.session_id = session.session_id.clone();
                                let agent_name = if self.agent_model.is_empty() { None } else { Some(self.agent_model.clone()) };
                                self.start_agent(agent_name.as_deref()).await.map_err(|e| {
                                    io::Error::other(format!("Failed to start agent: {}", e))
                                })?;

                                // Send the restored trace to the agent
                                if let Some(ref agent) = self.agent {
                                    let _ = agent.controller.send_trace(session.trace.clone()).await;
                                }
                                self.input.alert_msg(&format!("Session {} restored", &session.session_id[..8]), Duration::from_secs(2));
                            } else {
                                self.input.alert_msg("Invalid session number", Duration::from_secs(2));
                            }
                        } else {
                            // List all sessions
                            let mut msg = String::from("Saved sessions:\n");
                            for (i, s) in sessions.iter().enumerate() {
                                let preview = s.trace.first()
                                    .map(|m| {
                                        match m {
                                            openai_dive::v1::resources::chat::ChatMessage::User { content, .. } => {
                                                match content {
                                                    openai_dive::v1::resources::chat::ChatMessageContent::Text(t) => t.chars().take(50).collect::<String>(),
                                                    _ => "(multimedia)".to_string(),
                                                }
                                            }
                                            _ => "(no user message)".to_string(),
                                        }
                                    })
                                    .unwrap_or_else(|| "(empty)".to_string());
                                msg.push_str(&format!("  {} - {} ... ({})\n", i + 1, &s.session_id[..8], preview));
                            }
                            msg.push_str("\nUse /restore <number> to restore a session");
                            self.input.alert_msg(&msg, Duration::from_secs(10));
                        }
                    }
                    Ok(_) => {
                        self.input.alert_msg("No saved sessions found", Duration::from_secs(2));
                    }
                    Err(e) => {
                        self.input.alert_msg(&format!("Failed to list sessions: {}", e), Duration::from_secs(3));
                    }
                }
            }
            _ => {
                self.input.alert_msg("command unknown", Duration::from_secs(1));
            }
        }
        Ok(())
    }
}
