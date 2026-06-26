use std::io::{self, stdout};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::{layout::Rect, prelude::CrosstermBackend, Frame, Terminal};

pub trait Modal {
    type Output;
    fn draw(&mut self, frame: &mut Frame, area: Rect);
    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> Option<Self::Output>;
}

pub async fn run_alternate_screen<M: Modal>(modal: &mut M) -> io::Result<M::Output> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut reader = event::EventStream::new();

    let result = async {
        loop {
            terminal.draw(|frame| {
                modal.draw(frame, frame.area());
            })?;

            if let Some(Ok(event)) = reader.next().await {
                match event {
                    Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                        if let Some(result) = modal.handle_key_event(key_event) {
                            return Ok(result);
                        }
                    }
                    Event::Resize(..) => {}
                    _ => {}
                }
            }
        }
    }
    .await;

    let _ = execute!(stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();

    result
}
