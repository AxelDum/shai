#![allow(clippy::collapsible_if)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::collapsible_match)]
use std::io::{self};
use std::sync::Arc;

use ansi_to_tui::IntoText;
use chrono::Utc;
use cli_clipboard::ClipboardProvider;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{self, disable_raw_mode};
use futures::{future::FutureExt, StreamExt};
use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent};
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Stylize;
use ratatui::Terminal;
use ratatui::{
    layout::{Constraint, Layout},
    widgets::Widget,
    Frame, TerminalOptions, Viewport,
};
use shai_core::agent::builder::AgentBuilder;
use shai_core::agent::events::PermissionRequest;
use shai_core::agent::output::PrettyFormatter;
use shai_core::agent::{Agent, AgentController, AgentEvent, PublicAgentState};
use shai_core::config::agent::AgentConfig;
use shai_core::config::config::ShaiConfig;
use shai_core::runners::coder::coder::coder;
use shai_core::tools::ToolResult;
use shai_llm::ToolCallMethod;
use std::io::Write;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};
use tracing::{debug, warn};

use super::command::CommandRegistry;
use super::history::ConversationHistory;
use super::input::{AgentMode, UserAction};
use super::mcp_manager::McpManager;
use super::perm::PermissionModalAction;
use super::perm_manager::PermissionManager;
use super::session_manager::SessionManager;
use super::shortcuts::Shortcuts;
use super::session_picker::{SessionPicker, SessionPickerAction};
use super::statusbar::StatusBar;
use super::theme::Theme;
use super::token_counter::TokenCounter;
use super::tool_tracker::ToolTracker;
use super::viewer::AlternateScreenViewer;
use crate::tui::input::InputArea;
use crate::tui::perm::PermissionWidget;
use crate::tui::perm_alt_screen::AlternateScreenPermissionModal;

pub enum AppModalState<'a> {
    InputShown,
    PermissionModal { widget: PermissionWidget<'a> },
}

pub struct AppRunningAgent {
    pub(crate) handle: JoinHandle<()>,
    pub(crate) events: broadcast::Receiver<AgentEvent>,
    pub(crate) controller: AgentController,
    pub(crate) tools: Vec<(String, String)>,
}

pub struct App<'a> {
    pub(crate) terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,

    pub(crate) agent: Option<AppRunningAgent>,
    pub(crate) custom_agent: Option<Box<dyn Agent>>,

    pub(crate) state: AppModalState<'a>,
    pub(crate) formatter: PrettyFormatter, // streaming log formatter
    pub(crate) tool_tracker: ToolTracker,
    pub(crate) input: InputArea<'a>, // input text
    pub(crate) command_registry: CommandRegistry,
    pub(crate) shortcuts: Shortcuts,
    pub(crate) mcp_manager: McpManager,
    pub(crate) exit: bool,
    pub(crate) permission_manager: PermissionManager,

    pub(crate) token_counter: TokenCounter,

    pub(crate) theme: Theme, // UI theme (dark/light)
    pub(crate) status_bar: StatusBar,
    pub(crate) history: ConversationHistory,

    // Agent metadata for status bar
    pub(crate) agent_model: String,
    pub(crate) agent_provider: String,
    pub(crate) agent_name: Option<String>,

    // Session persistence
    pub(crate) session_manager: SessionManager,

    // Session picker modal
    pub(crate) session_picker: Option<SessionPicker>,
}

// Agent-related Internals
impl App<'_> {
    pub async fn start_agent(
        &mut self,
        agent_name: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut agent: Box<dyn Agent> = if let Some(agent_name) = agent_name {
            // Load custom agent config
            let config = AgentConfig::load(agent_name)?;

            // Store agent metadata for status bar
            self.agent_model = config.llm_provider.model.clone();
            self.agent_provider = config.llm_provider.provider.clone();
            self.agent_name = Some(agent_name.to_string());

            // Create agent from config
            let agent_builder = AgentBuilder::from_config(config).await?;
            self.mcp_manager.set_servers(agent_builder.mcp_status.clone());
            Box::new(agent_builder.build())
        } else {
            // Use default coder agent
            let (llm, model) = ShaiConfig::get_llm().await?;

            // Store agent metadata for status bar
            self.agent_model = model.clone();
            self.agent_provider = llm.provider().name().to_string();
            self.agent_name = None;

            Box::new(coder(Arc::new(llm), model))
        };

        let banner = if let Some(name) = agent_name {
            format!(
                "\x1b[2m░ agent {} - {} on {}\x1b[0m",
                name, self.agent_model, self.agent_provider
            )
        } else {
            format!(
                "\x1b[2m░ {} on {}\x1b[0m",
                self.agent_model, self.agent_provider
            )
        };

        // Get Agent I/O
        let controller = agent.controller();
        let events = agent.watch();

        // Run the agent in background
        let handle = tokio::spawn(async move {
            match agent.run().await {
                Ok(result) => debug!(target: "agent::loop", "Agent completed: {:?}", result),
                Err(error) => warn!(target: "agent::loop", "Agent failed: {}", error),
            }
        });

        self.agent = Some(AppRunningAgent {
            handle,
            controller: controller.clone(),
            events,
            tools: Vec::new(),
        });

        // Restore saved active prompts
        let saved_prompts = shai_core::tools::prompts::load_active_prompts_from_disk();
        if !saved_prompts.is_empty() {
            let _ = controller.set_active_prompts(saved_prompts).await;
        }

        // Update status bar with agent metadata
        self.status_bar.set_model(&self.agent_model);
        self.status_bar.set_provider(&self.agent_provider);

        self.refresh_status_bar();

        Ok(banner)
    }

    async fn receive_agent_event(&mut self) -> Option<AgentEvent> {
        if let Some(ref mut agent) = self.agent {
            agent.events.recv().await.ok()
        } else {
            None
        }
    }

    /// Render a restored trace into the TUI terminal and conversation history.
    /// This is used by /restore to display the loaded conversation.
    pub(crate) fn render_restored_trace(
        &mut self,
        trace: &[openai_dive::v1::resources::chat::ChatMessage],
    ) {
        use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent};

        for message in trace {
            let formatted = match message {
                ChatMessage::User { content, .. } => match content {
                    ChatMessageContent::Text(text) => {
                        self.formatter.format_event(&AgentEvent::UserInput {
                            input: text.clone(),
                        })
                    }
                    _ => None,
                },
                ChatMessage::Assistant { content, .. } => {
                    if let Some(ChatMessageContent::Text(text)) = content {
                        if text.trim().is_empty() {
                            None
                        } else {
                            Some(format!("\n● {}", text))
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(text) = formatted {
                self.history.add_text(&text);
            }
        }
    }

    /// Restore a session by ID: load it from disk, send the trace to the agent,
    /// and render it in the TUI.
    pub(crate) async fn restore_session(&mut self, session_id: &str) -> io::Result<()> {
        let session =
            shai_core::session::SessionPersist::load_session(session_id).map_err(|e| {
                io::Error::other(format!("Failed to load session {}: {}", session_id, e))
            })?;

        self.session_manager.set_session_id(&session.session_id);

        // Send the trace to the agent (without starting to think)
        if let Some(ref agent) = self.agent {
            let _ = agent.controller.load_trace(session.trace.clone()).await;
        }

        // Render the trace into the TUI
        self.render_restored_trace(&session.trace);
        self.history.scroll_to_bottom();
        self.refresh_status_bar();

        Ok(())
    }

    async fn handle_agent_event(&mut self, event: AgentEvent) -> io::Result<()> {
        // Update agent state
        if let AgentEvent::StatusChanged { new_status, .. } = &event {
            self.input
                .set_agent_running(!matches!(new_status, PublicAgentState::Paused));
            self.status_bar
                .set_agent_mode(&format!("{:?}", self.input.agent_mode()));
            // Save session to disk when agent pauses (finishes processing)
            if matches!(new_status, PublicAgentState::Paused) {
                if let Some(ref agent_ref) = self.agent {
                    if let Ok(trace) = agent_ref.controller.get_trace().await {
                        let sid = self.session_manager.session_id().to_string();
                        tokio::spawn(async move {
                            if let Err(e) =
                                shai_core::session::SessionPersist::save_session(&sid, trace)
                            {
                                tracing::warn!("Failed to save session {}: {}", sid, e);
                            }
                        });
                    }
                }
            }
        }

        // updated inprogress list
        if let AgentEvent::ToolCallStarted { call, .. } = &event {
            self.tool_tracker.start_tool(call.clone());
        }
        if let AgentEvent::ToolCallCompleted { call, result, .. } = &event {
            self.tool_tracker.complete_tool(call, result);
        }

        // Format and display event
        if let Some(formatted) = self.formatter.format_event(&event) {
            // Track last assistant response for Ctrl+K copy
            if let AgentEvent::BrainResult {
                thought: Ok(ChatMessage::Assistant { content, .. }),
                ..
            } = &event
            {
                if let Some(ChatMessageContent::Text(text)) = content {
                    if !text.trim().is_empty() {
                        self.session_manager.set_last_assistant_response(text);
                    }
                }
            }

            // Add to conversation history (rendered by draw_ui)
            self.history.add_text(&formatted);
            self.history.scroll_to_bottom();
        }

        // Handle permission requests - just add to queue
        if let AgentEvent::PermissionRequired {
            request_id,
            request,
        } = &event
        {
            self.permission_manager
                .push(request_id.clone(), request.clone());
        }

        // Handle token usage tracking
        if let AgentEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cached_tokens,
        } = &event
        {
            self.token_counter
                .add(*input_tokens, *output_tokens, *cached_tokens);
            self.status_bar.set_tokens(
                self.token_counter.input_tokens(),
                self.token_counter.output_tokens(),
            );
        }

        // Handle error events - display inline in red
        if let AgentEvent::Error { error } = &event {
            let error_msg = format!("\x1b[31m✘ Error: {}\x1b[0m", error);
            self.history.add_text(&error_msg);
            self.history.scroll_to_bottom();
        }

        self.refresh_status_bar();

        Ok(())
    }

    /// Refresh the status bar with current working directory and git branch.
    fn refresh_status_bar(&mut self) {
        if let Ok(cwd) = std::env::current_dir() {
            self.status_bar
                .set_location(&cwd.to_string_lossy().to_string());
        }
        if let Ok(output) = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
        {
            if output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout);
                self.status_bar.set_git_branch(branch.trim());
            }
        }
    }
}

// UI-related Internals
impl App<'_> {
    pub fn new() -> Self {
        let theme = Theme::from_env(); // Read from SHAI_TUI_THEME env var
        let palette = theme.palette();
        let shortcuts = Shortcuts::load();
        let mut input = InputArea::new(palette);
        input.set_shortcuts(
            shortcuts.cancel_task().clone(),
            shortcuts.clear_input().clone(),
            shortcuts.paste().clone(),
        );

        Self {
            terminal: None,
            agent: None,
            custom_agent: None,
            formatter: PrettyFormatter::new(),
            state: AppModalState::InputShown,
            input,
            command_registry: CommandRegistry::new(),
            shortcuts,
            exit: false,
            tool_tracker: ToolTracker::new(),
            permission_manager: PermissionManager::new(),
            token_counter: TokenCounter::new(),
            theme,
            status_bar: StatusBar::new(theme),
            history: ConversationHistory::new(),
            agent_model: String::new(),
            agent_provider: String::new(),
            agent_name: None,
            session_manager: SessionManager::new(),
            mcp_manager: McpManager::new(),
            session_picker: None,
        }
    }

    /// Display a transient notification in the status bar.
    pub fn notify(&mut self, msg: &str, duration: std::time::Duration) {
        self.status_bar.set_notification(msg, duration);
    }

    pub async fn run(
        &mut self,
        agent_name: Option<String>,
        restore_session_id: Option<String>,
    ) -> io::Result<()> {
        let x = self.try_run(agent_name, restore_session_id).await;

        // Disable mouse capture and restore keyboard protocol
        let _ = execute!(
            std::io::stdout(),
            crossterm::event::DisableMouseCapture,
            crossterm::event::PopKeyboardEnhancementFlags,
        );
        std::io::stdout().flush().ok();
        let _ = disable_raw_mode();

        if let Err(e) = x {
            // Simply print a newline to move cursor to next line and beginning
            println!();
            eprintln!("{}\r\n", e);
        }

        println!();
        println!();
        Ok(())
    }

    async fn try_run(
        &mut self,
        agent_name: Option<String>,
        restore_session_id: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Start the agent (custom or default)
        let agent_name_ref = agent_name.as_deref();
        let banner =
            self.start_agent(agent_name_ref)
                .await
                .map_err(|e| -> Box<dyn std::error::Error> {
                    if let Some(name) = agent_name_ref {
                        format!("could not start custom agent '{}': {}", name, e).into()
                    } else {
                        "could not start shai agent, run shai auth first"
                            .to_string()
                            .into()
                    }
                })?;

        // create terminal
        self.terminal = Some(ratatui::init_with_options(TerminalOptions {
            viewport: Viewport::Fullscreen,
        }));

        // Enable mouse capture for scroll wheel support
        execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;

        // Clear the alternate screen so previous shell output doesn't show through
        if let Some(ref mut terminal) = self.terminal {
            terminal.clear()?;
        }

        // Render the startup banner into the TUI history
        if !banner.is_empty() {
            self.history.add_text(&banner);
        }

        std::io::stdout().flush().ok();

        // If restoring a session, load it now (after terminal is initialized)
        if let Some(session_id) = restore_session_id {
            self.restore_session(&session_id).await?;
        }

        // Create a timer for animation updates
        let mut animation_timer = interval(Duration::from_millis(100));
        let mut reader = crossterm::event::EventStream::new();

        while !self.exit {
            // Always draw the UI first
            self.draw_ui().map_err(|_| -> Box<dyn std::error::Error> {
                "oops... (x_x)'".to_string().into()
            })?;

            tokio::select! {
                // Handle agent events (only when not in permission modal)
                agent_event = self.receive_agent_event(), if self.agent.is_some() => {
                    if let Some(event) = agent_event {
                        self.handle_agent_event(event).await?;
                    }
                }

                // Handle keyboard input
                crossterm_event = reader.next() => {
                    if let Some(Ok(event)) = crossterm_event {
                        self.handle_crossterm_event(event).await?;
                    }
                }

                // Handle animation timer (fires when animating OR when checking for pending enter)
                _ = animation_timer.tick() => {
                    // Check for pending enter timeout
                    if let Some(action) = self.input.check_pending_enter() {
                        self.handle_user_action(action).await?;
                    }
                    // Timer ticked, UI will be redrawn in next iteration
                }
            }

            // Check permission queue and update state
            self.check_permission_queue().await?;
        }
        Ok(())
    }

    async fn handle_crossterm_event(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Resize(..) => {
                if let Some(ref mut terminal) = self.terminal {
                    terminal.clear()?;
                }
            }
            Event::Mouse(mouse_event) => match mouse_event.kind {
                MouseEventKind::ScrollUp => {
                    self.history.scroll_up(3);
                }
                MouseEventKind::ScrollDown => {
                    self.history.scroll_down(3);
                }
                _ => {}
            },
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                // Handle scroll keys without resetting scroll position
                let ctrl = key_event
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL);
                match key_event.code {
                    KeyCode::PageUp => {
                        self.history.scroll_up(3);
                        return Ok(());
                    }
                    KeyCode::PageDown => {
                        self.history.scroll_down(3);
                        return Ok(());
                    }
                    KeyCode::Up if ctrl => {
                        self.history.scroll_up(3);
                        return Ok(());
                    }
                    KeyCode::Down if ctrl => {
                        self.history.scroll_down(3);
                        return Ok(());
                    }
                    _ => {}
                }
                // Any other key resets scroll to bottom
                self.history.scroll_to_bottom();
                self.handle_key_event(key_event).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_session_picker_key(&mut self, key_event: KeyEvent) -> io::Result<()> {
        use super::session_picker::SessionPickerAction;

        let picker = self.session_picker.as_mut().unwrap();
        match picker.handle_key_event(key_event) {
            Some(SessionPickerAction::Selected(idx)) => {
                // Retrieve the session data before dropping the picker
                let sessions = shai_core::session::SessionPersist::list_sessions();
                self.session_picker = None;
                match sessions {
                    Ok(sessions) if idx < sessions.len() => {
                        let session = &sessions[idx];
                        let session_id = session.session_id.clone();
                        let trace = session.trace.clone();

                        self.input.alert_msg(
                            &format!("Restoring session {}...", &session_id[..8]),
                            Duration::from_secs(2),
                        );

                        // Drop existing agent
                        if let Some(agent) = self.agent.take() {
                            let _ = agent.controller.terminate().await;
                        }

                        // Start new agent
                        let agent_name = self.agent_name.clone();
                        self.start_agent(agent_name.as_deref()).await.map_err(|e| {
                            io::Error::other(format!("Failed to start agent: {}", e))
                        })?;

                        // Load the trace into the agent
                        if let Some(ref agent) = self.agent {
                            let _ = agent.controller.load_trace(trace.clone()).await;
                        }

                        // Render the trace into the TUI
                        self.session_manager.set_session_id(&session_id);
                        self.render_restored_trace(&trace);

                        self.input.alert_msg(
                            &format!("Session {} restored", &session_id[..8]),
                            Duration::from_secs(2),
                        );
                    }
                    _ => {
                        self.input
                            .alert_msg("Failed to restore session", Duration::from_secs(2));
                    }
                }
            }
            Some(SessionPickerAction::Cancelled) => {
                self.session_picker = None;
            }
            None => {}
        }
        Ok(())
    }

    async fn handle_key_event(&mut self, key_event: KeyEvent) -> io::Result<()> {
        if self.shortcuts.matches(&key_event, self.shortcuts.exit()) {
            self.exit = true;
            return Ok(());
        }

        // Handle session picker modal
        if self.session_picker.is_some() {
            return self.handle_session_picker_key(key_event).await;
        }

        // Handle theme toggle
        if self.shortcuts.matches(&key_event, self.shortcuts.toggle_theme()) {
            self.theme.toggle();
            let new_palette = self.theme.palette();
            self.input.set_palette(new_palette);
            return Ok(());
        }

        // Handle clear screen
        if self.shortcuts.matches(&key_event, self.shortcuts.clear_screen()) {
            if let Some(ref mut terminal) = self.terminal {
                terminal.clear()?;
            }
            self.history.clear();
            return Ok(());
        }

        // Handle regenerate
        if self.shortcuts.matches(&key_event, self.shortcuts.regenerate()) {
            if let Some(ref agent) = self.agent {
                let _ = agent.controller.regenerate().await;
                self.notify("Regenerating last response...", Duration::from_secs(2));
            }
            return Ok(());
        }

        // Handle copy response
        if self.shortcuts.matches(&key_event, self.shortcuts.copy_response()) {
            if !self.session_manager.last_assistant_response().is_empty() {
                if let Ok(mut ctx) = cli_clipboard::ClipboardContext::new() {
                    let _ = ctx
                        .set_contents(self.session_manager.last_assistant_response().to_string());
                    self.notify("Copied last response to clipboard", Duration::from_secs(2));
                }
            } else {
                self.notify("No assistant response to copy", Duration::from_secs(2));
            }
            return Ok(());
        }

        // Handle cycle agent mode
        if self.shortcuts.matches(&key_event, self.shortcuts.cycle_agent_mode()) {
            let mode = self.input.cycle_agent_mode();
            self.status_bar.set_agent_mode(&format!("{:?}", mode));
            if let Some(ref agent) = self.agent {
                match mode {
                    AgentMode::Plan => {
                        let _ = agent.controller.plan_mode().await;
                        let _ = agent.controller.no_sudo().await;
                    }
                    AgentMode::Manual => {
                        let _ = agent.controller.no_plan_mode().await;
                        let _ = agent.controller.no_sudo().await;
                    }
                    AgentMode::Auto => {
                        let _ = agent.controller.no_plan_mode().await;
                        let _ = agent.controller.sudo().await;
                    }
                }
            }
            return Ok(());
        }

        // Handle expand tool
        if self.shortcuts.matches(&key_event, self.shortcuts.expand_tool()) {
            if let Some(output) = self.tool_tracker.last_output().map(|s| s.to_string()) {
                let file_path = self.tool_tracker.last_file_path().map(|s| s.to_string());
                let mut viewer = AlternateScreenViewer::new(output, file_path);
                let _ = viewer.run().await;
            }
            return Ok(());
        }

        // Handle session picker
        if self.shortcuts.matches(&key_event, self.shortcuts.session_picker()) {
            let sessions = shai_core::session::SessionPersist::list_sessions().unwrap_or_default();
            self.session_picker = Some(SessionPicker::new(sessions, self.theme.palette()));
            return Ok(());
        }

        // Handle prompt picker
        if self.shortcuts.matches(&key_event, self.shortcuts.prompt_picker()) {
            let prompts = shai_core::tools::prompts::discover_prompts();
            let active = shai_core::tools::prompts::load_active_prompts_from_disk();
            let mut picker = crate::tui::prompt_picker::PromptPicker::new(
                prompts,
                &active,
                self.theme.palette(),
            );
            match picker.run().await {
                Ok(crate::tui::prompt_picker::PromptPickerAction::Selected(selected)) => {
                    if let Some(ref agent) = self.agent {
                        let _ = agent.controller.set_active_prompts(selected.clone()).await;
                        let _ = shai_core::tools::prompts::save_active_prompts(&selected);
                        self.notify("System prompts updated", std::time::Duration::from_secs(2));
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        match &mut self.state {
            AppModalState::InputShown => {
                let action = self.input.handle_event(key_event).await;
                self.handle_user_action(action).await?;
            }
            AppModalState::PermissionModal { widget } => {
                let action = widget.handle_key_event(key_event);
                self.handle_permission_action(action).await?;
            }
        }
        Ok(())
    }

    async fn handle_permission_action(&mut self, action: PermissionModalAction) -> io::Result<()> {
        match action {
            PermissionModalAction::Response { request_id, choice } => {
                // Send response to agent
                if let Some(ref agent) = self.agent {
                    if matches!(
                        choice,
                        shai_core::agent::events::PermissionResponse::AllowAlways
                    ) {
                        let _ = agent.controller.sudo().await;
                        self.input.set_agent_mode(AgentMode::Auto);
                        self.status_bar.set_agent_mode("Auto");
                    }
                    if let Err(e) = agent
                        .controller
                        .response_permission_request(request_id, choice)
                        .await
                    {
                        self.notify(
                            "channel with agent closed. Please restart the app",
                            Duration::from_secs(3),
                        );
                    }
                }

                // Remove the completed permission from queue
                self.permission_manager.pop();

                // Go back to InputShown so next check_permission_queue will show next permission
                self.state = AppModalState::InputShown;
            }
            PermissionModalAction::Nope => {}
        }
        Ok(())
    }

    async fn check_permission_queue(&mut self) -> io::Result<()> {
        match &self.state {
            AppModalState::InputShown if !self.permission_manager.is_empty() => {
                let (request_id, request) = self.permission_manager.front().unwrap();
                let palette = self.theme.palette();
                let widget = PermissionWidget::new(
                    request_id.clone(),
                    request.clone(),
                    self.permission_manager.len(),
                    palette,
                );

                let terminal_height = self
                    .terminal
                    .as_ref()
                    .and_then(|t| t.size().ok())
                    .map(|s| s.height)
                    .unwrap_or(24);

                if widget.height() > terminal_height.saturating_sub(5) {
                    // Use alternate screen for large modals
                    let action =
                        match AlternateScreenPermissionModal::new(&widget, palette) {
                            Ok(mut modal) => modal.run().await.unwrap_or_else(|_| {
                                PermissionModalAction::Response {
                                    request_id: request_id.clone(),
                                    choice: shai_core::agent::PermissionResponse::Deny,
                                }
                            }),
                            Err(_) => PermissionModalAction::Response {
                                request_id: request_id.clone(),
                                choice: shai_core::agent::PermissionResponse::Deny,
                            },
                        };
                    self.handle_permission_action(action).await?;
                } else {
                    // Use inline modal for small modals
                    self.state = AppModalState::PermissionModal { widget };
                }
            }
            AppModalState::PermissionModal { .. } if self.permission_manager.is_empty() => {
                self.state = AppModalState::InputShown;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_user_action(&mut self, action: UserAction) -> io::Result<()> {
        match action {
            UserAction::Nope => {}
            UserAction::CancelTask => {
                if let Some(ref agent) = self.agent {
                    let _ = agent.controller.stop_current_task().await;
                    self.notify("Task cancelled", Duration::from_secs(1));
                }
            }
            UserAction::UserInput { input } => {
                if let Some(ref agent) = self.agent {
                    if let Err(e) = agent.controller.send_user_input(input.clone()).await {
                        self.notify(
                            "channel with agent closed. Please restart the app",
                            Duration::from_secs(3),
                        );
                    }
                }
            }
            UserAction::UserAppCommand { command } => {
                let _ = CommandRegistry::dispatch(&command, self).await;
            }
        }
        Ok(())
    }

    fn draw_ui(&mut self) -> io::Result<()> {
        let modal_height = match &self.state {
            AppModalState::InputShown => self.input.height(),
            AppModalState::PermissionModal { widget } => widget.height(),
        }
        .max(5);

        let running_tools_height = self.tool_tracker.len() as u16;

        if let Some(ref mut terminal) = self.terminal {
            terminal.draw(|frame| {
                let [_, history_area, _, tools_area, modal_area, statusbar_area] =
                    Layout::vertical([
                        Constraint::Length(1),                    // padding
                        Constraint::Fill(1),                      // conversation history
                        Constraint::Length(2),                    // bottom margin
                        Constraint::Length(running_tools_height), // running tools
                        Constraint::Length(modal_height),         // input or modal
                        Constraint::Length(1),                    // status bar
                    ])
                    .areas(frame.area());

                // draw conversation history
                self.history.draw(frame, history_area);

                // draw running tool with duration
                if !self.tool_tracker.is_empty() {
                    let layout: std::rc::Rc<[Rect]> =
                        Layout::vertical(vec![Constraint::Length(1); self.tool_tracker.len()])
                            .split(tools_area);
                    for ((tool_id, tc), &area) in self.tool_tracker.iter().zip(&*layout) {
                        let elapsed = self.tool_tracker.elapsed(tool_id).unwrap_or_default();
                        let secs = elapsed.as_secs();
                        let millis = elapsed.subsec_millis() / 10;
                        let tool_str = self.formatter.format_tool_running(tc);
                        let tool_with_time =
                            format!("{} ({:.1}s)", tool_str, secs as f64 + millis as f64 / 100.0);
                        frame.render_widget(tool_with_time.into_text().unwrap(), area);
                    }
                }

                // draw modal
                match &self.state {
                    AppModalState::InputShown => self.input.draw(frame, modal_area),
                    AppModalState::PermissionModal { widget } => widget.draw(frame, modal_area),
                }

                // draw status bar
                self.status_bar.draw(frame, statusbar_area);

                // draw session picker overlay if active
                if let Some(ref mut picker) = self.session_picker {
                    let picker_area = Rect {
                        x: 2,
                        y: 1,
                        width: frame.area().width.saturating_sub(4),
                        height: frame.area().height.saturating_sub(2),
                    };
                    picker.draw(frame, picker_area);
                }
            })?;
        }

        Ok(())
    }
}
