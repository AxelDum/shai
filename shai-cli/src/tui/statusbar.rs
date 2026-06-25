use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Widget,
    Frame,
};

use super::theme::Theme;

/// Format a token count into human-readable form (K, M, G)
fn format_tokens(n: u32) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}K", n as f64 / 1000.0)
    } else if n < 1_000_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else {
        format!("{:.1}G", n as f64 / 1_000_000_000.0)
    }
}

/// Maximum display width for the location path in the status bar
const MAX_LOCATION_WIDTH: usize = 30;

/// Shorten a filesystem path for display in the status bar.
/// Replaces the $HOME prefix with `~` and truncates the beginning if too long.
fn shorten_path(path: &str) -> String {
    // Replace home prefix with ~
    let home = std::env::var("HOME").unwrap_or_default();
    let display = if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };

    // Truncate from the left if still too long
    if display.len() <= MAX_LOCATION_WIDTH {
        return display;
    }

    let truncated = &display[display.len() - MAX_LOCATION_WIDTH + 3..];
    format!("...{}", truncated)
}

/// Information displayed in the persistent status bar
#[derive(Clone)]
pub struct StatusBarInfo {
    pub model: String,
    pub provider: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub location: String,
    pub git_branch: String,
    pub agent_mode: String,
}

pub struct StatusBar {
    info: StatusBarInfo,
    theme: Theme,
}

impl StatusBar {
    pub fn new(theme: Theme) -> Self {
        Self {
            info: StatusBarInfo {
                model: String::new(),
                provider: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                location: String::new(),
                git_branch: String::new(),
                agent_mode: String::new(),
            },
            theme,
        }
    }

    pub fn update(&mut self, info: StatusBarInfo) {
        self.info = info;
    }

    pub fn set_model(&mut self, model: &str) {
        self.info.model = model.to_string();
    }

    pub fn set_provider(&mut self, provider: &str) {
        self.info.provider = provider.to_string();
    }

    pub fn set_tokens(&mut self, input: u32, output: u32) {
        self.info.input_tokens = input;
        self.info.output_tokens = output;
    }

    pub fn set_location(&mut self, location: &str) {
        self.info.location = location.to_string();
    }

    pub fn set_git_branch(&mut self, branch: &str) {
        self.info.git_branch = branch.to_string();
    }

    pub fn set_agent_mode(&mut self, mode: &str) {
        self.info.agent_mode = mode.to_string();
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![
            Span::styled(
                format!(" {} ", self.info.provider),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                self.info.model.clone(),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
        ];

        // Location (shown after model if available)
        if !self.info.location.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!(" {} ", shorten_path(&self.info.location)),
                Style::default().fg(Color::Black).bg(Color::Yellow),
            ));
        }

        // Git branch (shown after location if available)
        if !self.info.git_branch.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!(" {} ", self.info.git_branch),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ));
        }

        // Agent mode (shown after git branch)
        if !self.info.agent_mode.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!(" {} ", self.info.agent_mode),
                Style::default().fg(Color::Black).bg(Color::Green),
            ));
        }

        // Right-aligned: tokens
        let total = self.info.input_tokens + self.info.output_tokens;
        let token_str = format!(
            " {} ↑{} ↓{} ",
            format_tokens(total),
            format_tokens(self.info.input_tokens),
            format_tokens(self.info.output_tokens),
        );

        let left_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let right_len = token_str.chars().count();
        let padding = area.width as usize;
        let spaces = padding.saturating_sub(left_len + right_len);
        let spacer = " ".repeat(spaces);

        spans.push(Span::styled(
            spacer,
            Style::default().bg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            token_str,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ));

        let line = Line::from(spans);
        f.render_widget(line, area);
    }
}
