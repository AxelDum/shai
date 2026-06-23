use ansi_to_tui::IntoText;
use ratatui::{layout::Rect, style::{Color, Style, Stylize}, symbols::border, text::{Line, Span}, widgets::{Block, Borders, Padding, Widget}, Frame};



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
            "  /tokens              display token usage",
            "  /theme [dark|light]  set or toggle theme",
            "",
            "  Shortcuts:",
            "  Ctrl+T               toggle dark/light theme",
            "  Ctrl+V               paste from clipboard",
            "  Alt+Enter            insert newline (multi-line input)",
        ].join("\n").to_string()
    }
}

impl HelpArea {
    pub fn height(&self) -> u16 {
        12 // content (3 general help lines + 1 blank + 1 header + 3 command lines + 1 blank + 4 shortcut lines)
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let helper_text = self.helper_msg();
        let x = helper_text.into_text().unwrap();
        let x = x.style(Style::default().fg(Color::DarkGray).dim());
        f.render_widget(
            x, 
            area
        );
    }
}
