use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Widget,
    Frame,
};

use super::theme::Theme;

/// Information displayed in the persistent status bar
#[derive(Clone)]
pub struct StatusBarInfo {
    pub model: String,
    pub provider: String,
    pub agent_state: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
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
                agent_state: "starting".to_string(),
                input_tokens: 0,
                output_tokens: 0,
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

    pub fn set_agent_state(&mut self, state: &str) {
        self.info.agent_state = state.to_string();
    }

    pub fn set_tokens(&mut self, input: u32, output: u32) {
        self.info.input_tokens = input;
        self.info.output_tokens = output;
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let palette = self.theme.palette();

        let mut spans = vec![
            Span::styled(
                format!(" {} ", self.info.provider),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                self.info.model.clone(),
                Style::default().fg(palette.input_text),
            ),
        ];

        // Right-aligned: agent state + tokens
        let state_str = format!("{} ", self.info.agent_state);
        let token_str = format!(
            " {} ↑{} ↓{} ",
            self.info.input_tokens + self.info.output_tokens,
            self.info.input_tokens,
            self.info.output_tokens,
        );

        let left_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let right_len = state_str.chars().count() + token_str.chars().count();
        let padding = area.width as usize;
        let spaces = padding.saturating_sub(left_len + right_len);
        let spacer = " ".repeat(spaces);

        spans.push(Span::raw(spacer));
        spans.push(Span::styled(
            state_str,
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            token_str,
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ));

        let line = Line::from(spans);
        f.render_widget(line, area);
    }
}
