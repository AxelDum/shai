use ansi_to_tui::IntoText;
use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Widget},
    Frame,
};

pub struct HelpArea;

impl HelpArea {
    fn helper_msg(&self) -> String {
        [
            "  ? to print help      tap esc twice to clear input",
            "  / for commands       tap esc while agent is running to cancel",
            "                       ctrl^c to exit",
            "",
            "  Available Commands:",
            "  /exit                exit from the tui",
            "  /tc <method>         set tool call method: [auto | fc | fc2 | so]",
            "  /temp <float>        set the sampling temperature",
            "  /tokens              display token usage",
            "  /theme [dark|light|toggle]  set or toggle theme",
            "  /restore [index|id]  restore a previous session",
            "  /latest               restore the most recent session",
            "  /skills              list available skills",
            "  /regenerate          regenerate the last response",
            "",
            "  Shortcuts:",
            "  Ctrl+T               toggle dark/light theme",
            "  Ctrl+L               clear screen / reset viewport",
            "  Ctrl+R               retry/regenerate last response",
            "  Ctrl+K               copy last assistant response to clipboard",
            "  Ctrl+V               paste from clipboard",
            "  Alt+Enter            insert newline (multi-line input)",
        ]
        .join("\n")
        .to_string()
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
