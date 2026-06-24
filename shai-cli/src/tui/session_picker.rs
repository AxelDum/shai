// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: OVH SAS

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    layout::{Constraint, Layout},
    prelude::CrosstermBackend,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Padding},
    Frame, Terminal,
};
use shai_core::session::SessionData;
use std::io::{self, stdout, Write};

use super::theme::ThemePalette;

/// Result of the session picker modal.
pub enum SessionPickerAction {
    /// User selected a session by index into the provided list.
    Selected(usize),
    /// User cancelled (Esc / Ctrl+C).
    Cancelled,
}

struct SessionPickerState {
    sessions: Vec<SessionData>,
    selected: usize,
    scroll_offset: usize,
}

impl SessionPickerState {
    fn new(sessions: Vec<SessionData>) -> Self {
        Self {
            sessions,
            selected: 0,
            scroll_offset: 0,
        }
    }
}

pub struct SessionPicker {
    state: SessionPickerState,
    palette: ThemePalette,
}

impl SessionPicker {
    pub fn new(sessions: Vec<SessionData>, palette: ThemePalette) -> Self {
        Self {
            state: SessionPickerState::new(sessions),
            palette,
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        let chunks = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Min(1),    // list
        ])
        .split(area);

        // Title
        let title = Line::from(vec![Span::styled(
            " Restore Session ",
            Style::default().fg(self.palette.suggestion_selected_fg).bold(),
        )]);
        frame.render_widget(title, chunks[0]);

        if self.state.sessions.is_empty() {
            let empty = Line::from(Span::styled(
                " No saved sessions found.",
                Style::default().fg(self.palette.placeholder),
            ));
            frame.render_widget(empty, chunks[1]);
            return;
        }

        let max_visible = area.height.saturating_sub(2) as usize;
        let selected = self.state.selected;

        // Adjust scroll_offset so the selected item is always visible
        let scroll_offset = if self.state.sessions.len() <= max_visible {
            0
        } else if selected < self.state.scroll_offset {
            selected
        } else if selected >= self.state.scroll_offset + max_visible {
            selected - max_visible + 1
        } else {
            self.state.scroll_offset
        };

        let visible_count = self.state.sessions.len().min(max_visible);
        let items: Vec<ListItem> = self
            .state
            .sessions
            .iter()
            .skip(scroll_offset)
            .take(visible_count)
            .enumerate()
            .map(|(i, session)| {
                let idx = scroll_offset + i;
                let name = session.name.as_deref().unwrap_or("unnamed");
                let id_short = &session.session_id[..8.min(session.session_id.len())];

                let line = Line::from(vec![
                    Span::styled(
                        format!("{:>2}. ", idx + 1),
                        Style::default().fg(self.palette.placeholder),
                    ),
                    Span::styled(
                        format!("{:<50} ", name),
                        Style::default().fg(self.palette.input_text),
                    ),
                    Span::styled(
                        format!("({})", id_short),
                        Style::default().fg(self.palette.placeholder),
                    ),
                ]);

                if idx == self.state.selected {
                    ListItem::new(line).style(Style::default().bg(self.palette.suggestion_selected_bg))
                } else {
                    ListItem::new(line)
                }
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .border_style(Style::default().fg(self.palette.border))
                    .padding(Padding::new(1, 1, 0, 0)),
            );

        frame.render_widget(list, chunks[1]);
    }

    pub async fn run(&mut self) -> io::Result<SessionPickerAction> {
        execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;

        let result = self.run_inner().await;

        let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
        let _ = stdout().flush();

        result
    }

    async fn run_inner(&mut self) -> io::Result<SessionPickerAction> {
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let mut reader = event::EventStream::new();

        loop {
            terminal.draw(|frame| {
                self.draw(frame);
            })?;

            if let Some(Ok(event)) = reader.next().await {
                match event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        // Ctrl+C cancels
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            return Ok(SessionPickerAction::Cancelled);
                        }
                        match key.code {
                            KeyCode::Up => {
                                if self.state.selected > 0 {
                                    self.state.selected -= 1;
                                }
                            }
                            KeyCode::Down => {
                                if self.state.selected + 1 < self.state.sessions.len() {
                                    self.state.selected += 1;
                                }
                            }
                            KeyCode::PageUp => {
                                self.state.selected = self.state.selected.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                self.state.selected = (self.state.selected + 10)
                                    .min(self.state.sessions.len().saturating_sub(1));
                            }
                            KeyCode::Home => {
                                self.state.selected = 0;
                            }
                            KeyCode::End => {
                                self.state.selected = self.state.sessions.len().saturating_sub(1);
                            }
                            KeyCode::Enter => {
                                if !self.state.sessions.is_empty() {
                                    return Ok(SessionPickerAction::Selected(self.state.selected));
                                }
                            }
                            KeyCode::Esc => {
                                return Ok(SessionPickerAction::Cancelled);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
