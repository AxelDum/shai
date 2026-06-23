use std::collections::VecDeque;

use ansi_to_tui::IntoText;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, Widget},
    Frame,
};

/// Maximum number of lines stored in the scrollback buffer
const MAX_SCROLLBACK_LINES: usize = 5000;

/// A single line in the conversation history
#[derive(Clone)]
struct ConversationLine {
    text: String,
}

pub struct ConversationHistory {
    /// All lines rendered so far (ANSI-formatted)
    lines: VecDeque<ConversationLine>,
    /// Scroll offset from the bottom (0 = latest)
    scroll_offset: usize,
}

impl ConversationHistory {
    pub fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(MAX_SCROLLBACK_LINES),
            scroll_offset: 0,
        }
    }

    /// Add rendered text lines to the history
    pub fn add_text(&mut self, text: &str) {
        for line in text.lines() {
            self.lines.push_back(ConversationLine {
                text: line.to_string(),
            });
            if self.lines.len() > MAX_SCROLLBACK_LINES {
                self.lines.pop_front();
            }
        }
    }

    /// Scroll up by `n` lines
    pub fn scroll_up(&mut self, n: usize) {
        let max_scroll = self.lines.len().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + n).min(max_scroll);
    }

    /// Scroll down by `n` lines
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Reset scroll to bottom (latest)
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Clear all history lines
    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll_offset = 0;
    }

    /// Check if scrolled to bottom
    pub fn at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// Render the conversation history into the given area
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        if self.lines.is_empty() {
            return;
        }

        let total_lines = self.lines.len();
        let visible_height = area.height as usize;

        if visible_height == 0 {
            return;
        }

        // Determine which lines to show
        let end = total_lines.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(visible_height);

        let lines_to_show: Vec<&ConversationLine> = self
            .lines
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();

        let combined: String = lines_to_show
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        if let Ok(text) = combined.into_text() {
            let paragraph = Paragraph::new(text);
            f.render_widget(paragraph, area);
        } else {
            // Fallback: render as plain text
            let paragraph = Paragraph::new(combined);
            f.render_widget(paragraph, area);
        }
    }
}
