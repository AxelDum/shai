use ansi_to_tui::IntoText;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation};
use ratatui::Frame as RataFrame;
use ratatui::Terminal;
use shai_core::tools::highlight::highlight_content;
use std::io::{self, stdout};

/// An expandable full-screen viewer for tool output with syntax highlighting.
///
/// Renders the given content in an alternate terminal screen with scroll support.
/// The user can navigate with arrow keys, Page Up/Down, and Home/End.
/// Press Escape or 'q' to exit.
pub struct AlternateScreenViewer {
    content: String,
    file_path: Option<String>,
}

impl AlternateScreenViewer {
    /// Create a new viewer for the given content.
    ///
    /// If `file_path` is provided, syntax highlighting will be applied based on the file extension.
    pub fn new(content: String, file_path: Option<String>) -> Self {
        Self { content, file_path }
    }

    /// Run the viewer in an alternate screen. Returns when the user exits.
    pub async fn run(&mut self) -> io::Result<()> {
        // Enter alternate screen
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let result = self.run_viewer().await;

        // Restore terminal
        let _ = execute!(stdout, LeaveAlternateScreen);
        let _ = disable_raw_mode();
        Ok(result?)
    }

    async fn run_viewer(&mut self) -> io::Result<()> {
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let mut reader = event::EventStream::new();
        let mut scroll_offset: usize = 0;

        // Apply syntax highlighting if we have a file path
        let display_content = match &self.file_path {
            Some(path) => highlight_content(&self.content, path),
            None => self.content.clone(),
        };

        loop {
            terminal.draw(|frame| self.draw(frame, &display_content, scroll_offset))?;

            if let Some(Ok(event)) = reader.next().await {
                match event {
                    Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                        match key_event.code {
                            KeyCode::Char('c')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                return Ok(());
                            }
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Up => {
                                scroll_offset = scroll_offset.saturating_sub(1);
                            }
                            KeyCode::Down => {
                                scroll_offset = scroll_offset.saturating_add(1);
                            }
                            KeyCode::PageUp => {
                                scroll_offset = scroll_offset.saturating_sub(20);
                            }
                            KeyCode::PageDown => {
                                scroll_offset = scroll_offset.saturating_add(20);
                            }
                            KeyCode::Home => {
                                scroll_offset = 0;
                            }
                            KeyCode::End => {
                                let total_lines = display_content.lines().count();
                                scroll_offset = total_lines;
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(..) => {}
                    _ => {}
                }
            }
        }
    }

    fn draw(&self, frame: &mut RataFrame, content: &str, scroll_offset: usize) {
        let area = frame.area();

        let title = match &self.file_path {
            Some(path) => format!(" {} ", path),
            None => " Tool Output ".to_string(),
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(format!(" {} ", title.trim()))
            .title_style(Style::default().add_modifier(Modifier::BOLD))
            .padding(Padding::new(1, 1, 1, 1));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Convert ANSI-colored content to ratatui Text
        let text = content.into_text().unwrap_or_else(|_| {
            Text::styled(content.to_string(), Style::default().fg(Color::White))
        });

        let total_lines = text.lines.len();
        let visible_height = inner.height as usize;

        // Clamp scroll offset
        let clamped_offset = if total_lines > visible_height {
            scroll_offset.min(total_lines.saturating_sub(visible_height))
        } else {
            0
        };

        let paragraph = Paragraph::new(text).scroll((clamped_offset as u16, 0));
        frame.render_widget(paragraph, inner);

        // Draw scrollbar if content is longer than visible area
        if total_lines > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None);
            let mut scrollbar_state =
                ratatui::widgets::ScrollbarState::new(total_lines).position(clamped_offset);
            frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
        }

        // Draw footer help
        let footer_area = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        let footer = Paragraph::new(" \u{2191}\u{2193} Scroll | PageUp/PageDown | q/Esc Close ")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, footer_area);
    }
}

impl Drop for AlternateScreenViewer {
    fn drop(&mut self) {
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
