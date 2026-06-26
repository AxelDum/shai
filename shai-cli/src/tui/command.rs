use ansi_to_tui::IntoText;
use shai_llm::ToolCallMethod;
use std::{io, time::Duration};

use super::session_picker::SessionPicker;
use super::theme::Theme;
use crate::tui::App;
use ratatui::widgets::Widget;

pub struct CommandDef {
    pub name: &'static str,
    pub description: &'static str,
    pub args: &'static [&'static str],
}

pub struct CommandRegistry {
    commands: Vec<CommandDef>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: vec![
                CommandDef {
                    name: "/exit",
                    description: "exit from the tui",
                    args: &[],
                },
                CommandDef {
                    name: "/tc",
                    description: "set the tool call method: [auto | fc | fc2 | so]",
                    args: &["method"],
                },
                CommandDef {
                    name: "/temp",
                    description: "set the sampling temperature (e.g. /temp 0.3)",
                    args: &["temperature"],
                },
                CommandDef {
                    name: "/tokens",
                    description: "display token usage (input/output)",
                    args: &[],
                },
                CommandDef {
                    name: "/theme",
                    description: "set theme: [dark | light | toggle]",
                    args: &["mode"],
                },
                CommandDef {
                    name: "/restore",
                    description: "restore a previous session",
                    args: &[],
                },
                CommandDef {
                    name: "/latest",
                    description: "restore the most recent session",
                    args: &[],
                },
                CommandDef {
                    name: "/skills",
                    description: "list available skills",
                    args: &[],
                },
                CommandDef {
                    name: "/regenerate",
                    description: "regenerate the last response",
                    args: &[],
                },
            ],
        }
    }

    pub fn commands(&self) -> &[CommandDef] {
        &self.commands
    }

    pub async fn dispatch(command: &str, app: &mut App<'_>) -> io::Result<()> {
        let mut parts = command.split_whitespace();
        let cmd = parts.next().unwrap();
        let args: Vec<&str> = parts.collect();

        match cmd {
            "/exit" => {
                app.exit = true;
            }
            "/tc" => {
                if let Some(ref agent) = app.agent {
                    match args.into_iter().next() {
                        Some("auto") => {
                            if let Ok(method) = agent
                                .controller
                                .set_method(Some(ToolCallMethod::Auto))
                                .await
                            {
                                app.notify(
                                    "llm will now try all method for tool calls",
                                    Duration::from_secs(3),
                                );
                                app.input.set_tool_call_method(method);
                            }
                        }
                        Some("fc") => {
                            if let Ok(method) = agent
                                .controller
                                .set_method(Some(ToolCallMethod::FunctionCall))
                                .await
                            {
                                app.notify(
                                    "llm will now use function calling api for tool calls",
                                    Duration::from_secs(3),
                                );
                                app.input.set_tool_call_method(method);
                            }
                        }
                        Some("fc2") => {
                            if let Ok(method) = agent
                                .controller
                                .set_method(Some(ToolCallMethod::FunctionCallRequired))
                                .await
                            {
                                app.notify("llm will now use function calling in required mode for tool calls", Duration::from_secs(3));
                                app.input.set_tool_call_method(method);
                            }
                        }
                        Some("so") => {
                            if let Ok(method) = agent
                                .controller
                                .set_method(Some(ToolCallMethod::StructuredOutput))
                                .await
                            {
                                app.notify(
                                    "llm will now use structured output for tool calls",
                                    Duration::from_secs(3),
                                );
                                app.input.set_tool_call_method(method);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "/regenerate" => {
                if let Some(ref agent) = app.agent {
                    let _ = agent.controller.regenerate().await;
                    app.notify("Regenerating last response...", Duration::from_secs(2));
                }
            }
            "/temp" => {
                if let Some(ref agent) = app.agent {
                    match args.into_iter().next() {
                        Some(temp_str) => match temp_str.parse::<f32>() {
                            Ok(temp) => match agent.controller.set_temperature(temp).await {
                                Ok(temp) => {
                                    app.notify(
                                        &format!("Temperature set to {:.1}", temp),
                                        Duration::from_secs(2),
                                    );
                                }
                                Err(e) => {
                                    app.notify(
                                        &format!("Failed to set temperature: {}", e),
                                        Duration::from_secs(3),
                                    );
                                }
                            },
                            Err(_) => {
                                app.notify("Usage: /temp <float>", Duration::from_secs(3));
                            }
                        },
                        None => {
                            app.notify("Usage: /temp <float>", Duration::from_secs(3));
                        }
                    }
                }
            }
            "/tokens" => {
                let msg = format!(
                    "Token Usage - Input: {}, Output: {}, Cached: {}, Total: {}",
                    app.token_counter.input_tokens(),
                    app.token_counter.output_tokens(),
                    app.token_counter.cached_tokens(),
                    app.token_counter.total()
                );
                app.notify(&msg, Duration::from_secs(5));
            }
            "/theme" => match args.into_iter().next() {
                Some("dark") => {
                    app.theme = Theme::Dark;
                    let new_palette = app.theme.palette();
                    app.input.set_palette(new_palette);
                    app.notify("Theme set to dark", Duration::from_secs(2));
                }
                Some("light") => {
                    app.theme = Theme::Light;
                    let new_palette = app.theme.palette();
                    app.input.set_palette(new_palette);
                    app.notify("Theme set to light", Duration::from_secs(2));
                }
                Some("toggle") => {
                    app.theme.toggle();
                    let new_palette = app.theme.palette();
                    app.input.set_palette(new_palette);
                    let theme_name = match app.theme {
                        Theme::Dark => "dark",
                        Theme::Light => "light",
                    };
                    app.notify(
                        &format!("Theme toggled to {}", theme_name),
                        Duration::from_secs(2),
                    );
                }
                _ => {
                    app.notify("Usage: /theme [dark|light|toggle]", Duration::from_secs(3));
                }
            },
            "/restore" => {
                let sessions = shai_core::session::SessionPersist::list_sessions();
                match sessions {
                    Ok(sessions) if !sessions.is_empty() => {
                        if let Some(arg) = args.into_iter().next() {
                            // Try to match by index (1-based) or by session_id prefix
                            let selected = arg
                                .parse::<usize>()
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
                                app.notify(
                                    &format!("Restoring session {}...", &session.session_id[..8]),
                                    Duration::from_secs(2),
                                );

                                // Drop existing agent
                                if let Some(agent) = app.agent.take() {
                                    let _ = agent.controller.terminate().await;
                                }

                                // Start new agent
                                let agent_name = app.agent_name.clone();
                                app.start_agent(agent_name.as_deref()).await.map_err(|e| {
                                    io::Error::other(format!("Failed to start agent: {}", e))
                                })?;

                                // Load the trace into the agent (without starting to think)
                                if let Some(ref agent) = app.agent {
                                    let _ =
                                        agent.controller.load_trace(session.trace.clone()).await;
                                }

                                // Render the trace into the TUI
                                app.session_manager.set_session_id(&session.session_id);
                                app.render_restored_trace(&session.trace);

                                app.notify(
                                    &format!("Session {} restored", &session.session_id[..8]),
                                    Duration::from_secs(2),
                                );
                            } else {
                                app.notify("Invalid session number", Duration::from_secs(2));
                            }
                        } else {
                            // Open the session picker modal
                            let palette = app.theme.palette();
                            app.session_picker =
                                Some(crate::tui::session_picker::SessionPicker::new(
                                    sessions.clone(),
                                    palette,
                                ));
                        }
                    }
                    Ok(_) => {
                        app.notify("No saved sessions found", Duration::from_secs(2));
                    }
                    Err(e) => {
                        app.notify(
                            &format!("Failed to list sessions: {}", e),
                            Duration::from_secs(3),
                        );
                    }
                }
            }
            "/latest" => {
                match shai_core::session::SessionPersist::list_sessions() {
                    Ok(sessions) if !sessions.is_empty() => {
                        let session = &sessions[0];
                        app.notify(
                            &format!("Restoring session {}...", &session.session_id[..8]),
                            Duration::from_secs(2),
                        );

                        // Drop existing agent
                        if let Some(agent) = app.agent.take() {
                            let _ = agent.controller.terminate().await;
                        }

                        // Start new agent
                        let agent_name = app.agent_name.clone();
                        app.start_agent(agent_name.as_deref()).await.map_err(|e| {
                            io::Error::other(format!("Failed to start agent: {}", e))
                        })?;

                        // Load the trace into the agent (without starting to think)
                        if let Some(ref agent) = app.agent {
                            let _ = agent.controller.load_trace(session.trace.clone()).await;
                        }

                        // Render the trace into the TUI
                        app.session_manager.set_session_id(&session.session_id);
                        app.render_restored_trace(&session.trace);

                        app.notify(
                            &format!("Session {} restored", &session.session_id[..8]),
                            Duration::from_secs(2),
                        );
                    }
                    Ok(_) => {
                        app.notify("No saved sessions found", Duration::from_secs(2));
                    }
                    Err(e) => {
                        app.notify(
                            &format!("Failed to list sessions: {}", e),
                            Duration::from_secs(3),
                        );
                    }
                }
            }
            "/skills" => {
                let skills = shai_core::tools::skills::discovery::discover_skills();
                if skills.is_empty() {
                    app.notify("No skills found.", Duration::from_secs(3));
                } else {
                    let mut msg = String::from("\x1b[1mAvailable skills:\x1b[0m\n");
                    for skill in &skills {
                        if skill.description.is_empty() {
                            msg.push_str(&format!("  \x1b[36m\u{2022}\x1b[0m {}\n", skill.name));
                        } else {
                            msg.push_str(&format!(
                                "  \x1b[36m\u{2022}\x1b[0m \x1b[1m{}\x1b[0m \u{2014} {}\n",
                                skill.name, skill.description
                            ));
                        }
                    }
                    if let Some(ref mut terminal) = app.terminal {
                        let wrapped = msg.into_text().unwrap();
                        let line_count = wrapped.lines.len() as u16;
                        terminal.clear()?;
                        terminal.insert_before(line_count, |buf| {
                            wrapped.render(buf.area, buf);
                        })?;
                        app.history.add_text(&msg);
                    }
                }
            }
            _ => {
                app.notify("command unknown", Duration::from_secs(1));
            }
        }
        Ok(())
    }
}
