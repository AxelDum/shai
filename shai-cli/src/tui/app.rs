#![allow(clippy::collapsible_if)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::collapsible_match)]
use std::io::{self};
use std::sync::Arc;
use std::time::Instant;

use ansi_to_tui::IntoText;
use chrono::Utc;
use cli_clipboard::ClipboardProvider;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, disable_raw_mode};
use futures::{future::FutureExt, StreamExt};
use openai_dive::v1::resources::chat::{ChatMessage, ChatMessageContent};
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Stylize;
use ratatui::text::Text;
use ratatui::Terminal;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
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
use shai_core::tools::{ToolCall, ToolResult};
use shai_llm::ToolCallMethod;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};

use super::history::ConversationHistory;
use super::input::UserAction;
use super::perm::PermissionModalAction;
use super::statusbar::StatusBar;
use super::theme::Theme;
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
}

pub struct App<'a> {
    pub(crate) terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    pub(crate) terminal_height: u16,

    pub(crate) agent: Option<AppRunningAgent>,
    pub(crate) custom_agent: Option<Box<dyn Agent>>,

    pub(crate) state: AppModalState<'a>,
    pub(crate) formatter: PrettyFormatter, // streaming log formatter
    pub(crate) running_tools: HashMap<String, ToolCall>, // tool_call_id -> ToolCall
    pub(crate) tool_start_times: HashMap<String, Instant>, // tool_call_id -> start time
    pub(crate) input: InputArea<'a>,       // input text
    pub(crate) commands: HashMap<(String, String), Vec<String>>,
    pub(crate) exit: bool,
    pub(crate) permission_queue: VecDeque<(String, PermissionRequest)>, // (request_id, request)

    pub(crate) total_input_tokens: u32,
    pub(crate) total_output_tokens: u32,
    pub(crate) total_cached_tokens: u32,

    pub(crate) theme: Theme, // UI theme (dark/light)
    pub(crate) status_bar: StatusBar,
    pub(crate) history: ConversationHistory,

    // Agent metadata for status bar
    pub(crate) agent_model: String,
    pub(crate) agent_provider: String,
    pub(crate) agent_name: Option<String>,

    // Session persistence
    pub(crate) session_id: String,
    pub(crate) last_assistant_response: String,
}

// Agent-related Internals
impl App<'_> {
    pub async fn start_agent(
        &mut self,
        agent_name: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // When the terminal is already initialized (e.g. during /restore), skip the
        // println! banner since writing directly to stdout corrupts the inline viewport.
        let suppress_banner = self.terminal.is_some();

        let mut agent: Box<dyn Agent> = if let Some(agent_name) = agent_name {
            // Load custom agent config
            let config = AgentConfig::load(agent_name)?;

            if !suppress_banner {
                println!(
                    "\x1b[2m░ agent {} - {} on {}\x1b[0m",
                    agent_name, config.llm_provider.model, config.llm_provider.provider
                );
            }

            // Store agent metadata for status bar
            self.agent_model = config.llm_provider.model.clone();
            self.agent_provider = config.llm_provider.provider.clone();
            self.agent_name = Some(agent_name.to_string());

            // Create agent from config
            let agent_builder = AgentBuilder::from_config(config).await?;
            Box::new(agent_builder.build())
        } else {
            // Use default coder agent
            let (llm, model) = ShaiConfig::get_llm().await?;

            if !suppress_banner {
                println!("\x1b[2m░ {} on {}\x1b[0m", model, llm.provider().name());
            }

            // Store agent metadata for status bar
            self.agent_model = model.clone();
            self.agent_provider = llm.provider().name().to_string();
            self.agent_name = None;

            Box::new(coder(Arc::new(llm), model))
        };

        // Get Agent I/O
        let controller = agent.controller();
        let events = agent.watch();

        // Run the agent in background
        let handle = tokio::spawn(async move {
            match agent.run().await {
                Ok(result) => eprintln!("Agent completed: {:?}", result),
                Err(error) => eprintln!("Agent failed: {:?}", error),
            }
        });

        self.agent = Some(AppRunningAgent {
            handle,
            controller,
            events,
        });

        // Update status bar with agent metadata
        self.status_bar.set_model(&self.agent_model);
        self.status_bar.set_provider(&self.agent_provider);

        Ok(())
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
                if let Some(ref mut terminal) = self.terminal {
                    let wrapped = text.into_text().unwrap();
                    let line_count = wrapped.lines.len() as u16;
                    terminal.clear().ok();
                    if let Err(e) = terminal.insert_before(line_count, |buf| {
                        wrapped.render(buf.area, buf);
                    }) {
                        eprintln!("[shai] Failed to insert restored trace: {}", e);
                    }
                    self.history.add_text(&text);
                }
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

        self.session_id = session.session_id.clone();

        // Send the trace to the agent (without starting to think)
        if let Some(ref agent) = self.agent {
            let _ = agent.controller.load_trace(session.trace.clone()).await;
        }

        // Render the trace into the TUI
        self.render_restored_trace(&session.trace);
        Ok(())
    }

    async fn handle_agent_event(&mut self, event: AgentEvent) -> io::Result<()> {
        // Update agent state
        if let AgentEvent::StatusChanged { new_status, .. } = &event {
            self.input
                .set_agent_running(!matches!(new_status, PublicAgentState::Paused));
            self.status_bar
                .set_agent_state(&format!("{:?}", new_status).to_lowercase());

            // Save session to disk when agent pauses (finishes processing)
            if matches!(new_status, PublicAgentState::Paused) {
                if let Some(ref agent_ref) = self.agent {
                    if let Ok(trace) = agent_ref.controller.get_trace().await {
                        let sid = self.session_id.clone();
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
            self.running_tools
                .insert(call.tool_call_id.clone(), call.clone());
            self.tool_start_times
                .insert(call.tool_call_id.clone(), Instant::now());
        }
        if let AgentEvent::ToolCallCompleted { call, .. } = &event {
            self.running_tools.remove(&call.tool_call_id);
            self.tool_start_times.remove(&call.tool_call_id);
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
                        self.last_assistant_response = text.clone();
                    }
                }
            }

            if let Some(ref mut terminal) = self.terminal {
                let wrapped = formatted.into_text().unwrap();
                let line_count = wrapped.lines.len() as u16;
                terminal.clear()?; // this is to avoid visual artifact
                terminal.insert_before(line_count, |buf| {
                    wrapped.render(buf.area, buf);
                })?;

                // Also add to conversation history
                self.history.add_text(&formatted);
            }
        }

        // Handle permission requests - just add to queue
        if let AgentEvent::PermissionRequired {
            request_id,
            request,
        } = &event
        {
            self.permission_queue
                .push_back((request_id.clone(), request.clone()));
        }

        // Handle token usage tracking
        if let AgentEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cached_tokens,
        } = &event
        {
            self.total_input_tokens += input_tokens;
            self.total_output_tokens += output_tokens;
            self.total_cached_tokens += cached_tokens;
            self.status_bar
                .set_tokens(self.total_input_tokens, self.total_output_tokens);

        }

        // Handle error events - display inline in red
        if let AgentEvent::Error { error } = &event {
            let error_msg = format!("\x1b[31m❌ Error: {}\x1b[0m", error);
            if let Some(ref mut terminal) = self.terminal {
                let wrapped = error_msg.into_text().unwrap();
                let line_count = wrapped.lines.len() as u16;
                terminal.clear()?;
                terminal.insert_before(line_count, |buf| {
                    wrapped.render(buf.area, buf);
                })?;
                self.history.add_text(&error_msg);
            }
        }

        Ok(())
    }
}

// UI-related Internals
impl App<'_> {
    pub fn new() -> Self {
        let theme = Theme::from_env(); // Read from SHAI_TUI_THEME env var
        let palette = theme.palette();

        Self {
            terminal: None,
            terminal_height: 5,
            agent: None,
            custom_agent: None,
            formatter: PrettyFormatter::new(),
            state: AppModalState::InputShown,
            input: InputArea::new(palette),
            commands: Self::list_command(),
            exit: false,
            running_tools: HashMap::new(),
            tool_start_times: HashMap::new(),
            permission_queue: VecDeque::new(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cached_tokens: 0,
            theme,
            status_bar: StatusBar::new(theme),
            history: ConversationHistory::new(),
            agent_model: String::new(),
            agent_provider: String::new(),
            agent_name: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            last_assistant_response: String::new(),
        }
    }

    pub async fn run(
        &mut self,
        agent_name: Option<String>,
        restore_session_id: Option<String>,
    ) -> io::Result<()> {
        let x = self.try_run(agent_name, restore_session_id).await;

        // Restore keyboard protocol
        let _ = execute!(
            std::io::stdout(),
            crossterm::event::PopKeyboardEnhancementFlags
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
            viewport: Viewport::Inline(8),
        }));

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
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_key_event(&mut self, key_event: KeyEvent) -> io::Result<()> {
        if (matches!(key_event.code, KeyCode::Char('c'))
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL))
            || (matches!(key_event.code, KeyCode::Char('d'))
                && key_event
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL))
        {
            self.exit = true;
            return Ok(());
        }

        // Handle theme toggle with Ctrl+T
        if matches!(key_event.code, KeyCode::Char('t'))
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            self.theme.toggle();
            let new_palette = self.theme.palette();
            self.input.set_palette(new_palette);
            return Ok(());
        }

        // Handle Ctrl+L — Clear screen / reset viewport
        if matches!(key_event.code, KeyCode::Char('l'))
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            if let Some(ref mut terminal) = self.terminal {
                terminal.clear()?;
            }
            self.history.clear();
            return Ok(());
        }

        // Handle Ctrl+R — Retry/regenerate last response
        if matches!(key_event.code, KeyCode::Char('r'))
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            if let Some(ref agent) = self.agent {
                let _ = agent.controller.stop_current_task().await;
                self.input
                    .alert_msg("Retrying last response...", Duration::from_secs(2));
            }
            return Ok(());
        }

        // Handle Ctrl+K — Copy last assistant response to clipboard
        if matches!(key_event.code, KeyCode::Char('k'))
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            if !self.last_assistant_response.is_empty() {
                if let Ok(mut ctx) = cli_clipboard::ClipboardContext::new() {
                    let _ = ctx.set_contents(self.last_assistant_response.clone());
                    self.input
                        .alert_msg("Copied last response to clipboard", Duration::from_secs(2));
                }
            } else {
                self.input
                    .alert_msg("No assistant response to copy", Duration::from_secs(2));
            }
            return Ok(());
        }

        // Handle Ctrl+P — Toggle auto-approve permissions (sudo mode)
        if matches!(key_event.code, KeyCode::Char('p'))
            && key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            if let Some(ref agent) = self.agent {
                let is_sudo = agent.controller.is_sudo().await.unwrap_or(false);
                if is_sudo {
                    let _ = agent.controller.no_sudo().await;
                    self.input
                        .alert_msg("Auto-approve disabled", Duration::from_secs(2));
                } else {
                    let _ = agent.controller.sudo().await;
                    self.input
                        .alert_msg("Auto-approve enabled", Duration::from_secs(2));
                }
            }
            return Ok(());
        }

        match &mut self.state {
            AppModalState::InputShown => {
                let action = self.input.handle_event(key_event).await;
                self.handle_user_action(action).await?;
            }
            AppModalState::PermissionModal { widget } => {
                let action = widget.handle_key_event(key_event).await;
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
                    }
                    if let Err(e) = agent
                        .controller
                        .response_permission_request(request_id, choice)
                        .await
                    {
                        self.input.alert_msg(
                            "channel with agent closed. Please restart the app",
                            Duration::from_secs(3),
                        );
                    }
                }

                // Remove the completed permission from queue
                self.permission_queue.pop_front();

                // Go back to InputShown so next check_permission_queue will show next permission
                self.state = AppModalState::InputShown;
            }
            PermissionModalAction::Nope => {}
        }
        Ok(())
    }

    async fn check_permission_queue(&mut self) -> io::Result<()> {
        match &self.state {
            AppModalState::InputShown if !self.permission_queue.is_empty() => {
                let (request_id, request) = self.permission_queue.front().unwrap();
                let palette = self.theme.palette();
                let widget = PermissionWidget::new(
                    request_id.clone(),
                    request.clone(),
                    self.permission_queue.len(),
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
                    if let Ok(mut modal) = AlternateScreenPermissionModal::new(&widget, palette) {
                        let action = modal.run().await.unwrap_or(PermissionModalAction::Nope);
                        self.handle_permission_action(action).await?;
                    }
                } else {
                    // Use inline modal for small modals
                    self.state = AppModalState::PermissionModal { widget };
                }
            }
            AppModalState::PermissionModal { .. } if self.permission_queue.is_empty() => {
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
                    self.input
                        .alert_msg("Task cancelled", Duration::from_secs(1));
                }
            }
            UserAction::UserInput { input } => {
                if let Some(ref agent) = self.agent {
                    if let Err(e) = agent.controller.send_user_input(input.clone()).await {
                        self.input.alert_msg(
                            "channel with agent closed. Please restart the app",
                            Duration::from_secs(3),
                        );
                    }
                }
            }
            UserAction::UserAppCommand { command } => {
                let _ = self.handle_app_command(&command).await;
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

        let running_tools_height = self.running_tools.len() as u16;
        let height = modal_height + 1 + running_tools_height + 1; // status bar

        if let Some(ref mut terminal) = self.terminal {
            if height != self.terminal_height {
                terminal.set_viewport_height(height + 1)?;
                self.terminal_height = height;
            }

            terminal.draw(|frame| {
                let [_, inprogress, modal, statusbar] = Layout::vertical([
                    Constraint::Length(1),                        // padding
                    Constraint::Length(running_tools_height + 1), // running tool (if any)
                    Constraint::Length(modal_height),             // input or modal
                    Constraint::Length(1),                        // status bar
                ])
                .areas(frame.area());

                // draw running tool with duration
                if !self.running_tools.is_empty() {
                    let layout: std::rc::Rc<[Rect]> =
                        Layout::vertical(vec![Constraint::Length(1); self.running_tools.len() + 1])
                            .split(inprogress);
                    for ((tool_id, tc), &area) in self.running_tools.iter().zip(&*layout) {
                        let elapsed = self
                            .tool_start_times
                            .get(tool_id)
                            .map(|t| t.elapsed())
                            .unwrap_or_default();
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
                    AppModalState::InputShown => self.input.draw(frame, modal),
                    AppModalState::PermissionModal { widget } => widget.draw(frame, modal),
                }

                // draw status bar
                self.status_bar.draw(frame, statusbar);
            })?;
        }
        Ok(())
    }
}
