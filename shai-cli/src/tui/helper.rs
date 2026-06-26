use ansi_to_tui::IntoText;
use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Widget},
    Frame,
};

use super::command::COMMANDS;
use shai_core::config::tui::{KeyBinding, TuiConfig};

pub struct HelpArea;

fn format_binding(binding: &KeyBinding) -> String {
    binding.to_string()
}

impl HelpArea {
    fn helper_msg(&self) -> String {
        let mut lines: Vec<String> = vec![
            "  ? to print help      tap esc twice to clear input".to_string(),
            "  / for commands       tap esc while agent is running to cancel".to_string(),
            "                       ctrl^c to exit".to_string(),
            String::new(),
            "  Available Commands:".to_string(),
        ];

        for cmd in COMMANDS.iter() {
            let args_suffix = if cmd.args.is_empty() {
                String::new()
            } else {
                format!(" <{}>", cmd.args.join("> <"))
            };
            lines.push(format!("  {}{}\t{}", cmd.name, args_suffix, cmd.description));
        }

        lines.push(String::new());
        lines.push("  Shortcuts:".to_string());

        let config = TuiConfig::default();
        let s = &config.shortcuts;
        let bindings: [(&str, &str); 6] = [
            ("toggle_theme", "toggle dark/light theme"),
            ("clear_screen", "clear screen / reset viewport"),
            ("regenerate", "retry/regenerate last response"),
            ("copy_response", "copy last assistant response to clipboard"),
            ("paste", "paste from clipboard"),
            ("cycle_agent_mode", "cycle agent mode (Plan/Manual/Auto)"),
        ];

        let get_binding = |field: &str| -> &KeyBinding {
            match field {
                "toggle_theme" => &s.toggle_theme,
                "clear_screen" => &s.clear_screen,
                "regenerate" => &s.regenerate,
                "copy_response" => &s.copy_response,
                "paste" => &s.paste,
                "cycle_agent_mode" => &s.cycle_agent_mode,
                _ => unreachable!(),
            }
        };

        for (field, desc) in bindings.iter() {
            lines.push(format!("  {:<20} {}", format_binding(get_binding(field)), desc));
        }

        lines.join("\n")
    }
}

impl HelpArea {
    pub fn height(&self) -> u16 {
        self.helper_msg().lines().count() as u16
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let helper_text = self.helper_msg();
        let x = helper_text.into_text().unwrap();
        let x = x.style(Style::default().fg(Color::White));
        f.render_widget(x, area);
    }
}
