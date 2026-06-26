use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use shai_core::config::tui::{KeyBinding, KeyCode as ConfigKeyCode, KeyModifiers as ConfigKeyModifiers, TuiConfig};

/// Wrapper around `TuiConfig` that provides convenient key-matching against `crossterm::event::KeyEvent`.
pub struct Shortcuts {
    config: TuiConfig,
}

impl Shortcuts {
    pub fn load() -> Self {
        let config = TuiConfig::load();
        Self { config }
    }

    pub fn config(&self) -> &TuiConfig {
        &self.config
    }

    pub fn toggle_theme(&self) -> &KeyBinding {
        &self.config.shortcuts.toggle_theme
    }

    pub fn exit(&self) -> &KeyBinding {
        &self.config.shortcuts.exit
    }

    pub fn cancel_task(&self) -> &KeyBinding {
        &self.config.shortcuts.cancel_task
    }

    pub fn clear_input(&self) -> &KeyBinding {
        &self.config.shortcuts.clear_input
    }

    pub fn paste(&self) -> &KeyBinding {
        &self.config.shortcuts.paste
    }

    pub fn clear_screen(&self) -> &KeyBinding {
        &self.config.shortcuts.clear_screen
    }

    pub fn regenerate(&self) -> &KeyBinding {
        &self.config.shortcuts.regenerate
    }

    pub fn copy_response(&self) -> &KeyBinding {
        &self.config.shortcuts.copy_response
    }

    pub fn expand_tool(&self) -> &KeyBinding {
        &self.config.shortcuts.expand_tool
    }

    pub fn session_picker(&self) -> &KeyBinding {
        &self.config.shortcuts.session_picker
    }

    pub fn prompt_picker(&self) -> &KeyBinding {
        &self.config.shortcuts.prompt_picker
    }

    pub fn cycle_agent_mode(&self) -> &KeyBinding {
        &self.config.shortcuts.cycle_agent_mode
    }

    pub fn matches(&self, key_event: &KeyEvent, binding: &KeyBinding) -> bool {
        key_event_to_binding(key_event) == *binding
    }
}

pub fn key_event_to_binding(event: &KeyEvent) -> KeyBinding {
    let code = match event.code {
        KeyCode::Char(c) => ConfigKeyCode::Char(c),
        KeyCode::Esc => ConfigKeyCode::Escape,
        KeyCode::Tab => ConfigKeyCode::Tab,
        KeyCode::Enter => ConfigKeyCode::Enter,
        KeyCode::Backspace => ConfigKeyCode::Backspace,
        KeyCode::Delete => ConfigKeyCode::Delete,
        KeyCode::Left => ConfigKeyCode::Left,
        KeyCode::Right => ConfigKeyCode::Right,
        KeyCode::Up => ConfigKeyCode::Up,
        KeyCode::Down => ConfigKeyCode::Down,
        KeyCode::PageUp => ConfigKeyCode::PageUp,
        KeyCode::PageDown => ConfigKeyCode::PageDown,
        KeyCode::Home => ConfigKeyCode::Home,
        KeyCode::End => ConfigKeyCode::End,
        _ => ConfigKeyCode::Space,
    };

    let mut modifiers = ConfigKeyModifiers::NONE;
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        modifiers = modifiers | ConfigKeyModifiers::SHIFT;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        modifiers = modifiers | ConfigKeyModifiers::CONTROL;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        modifiers = modifiers | ConfigKeyModifiers::ALT;
    }
    if event.modifiers.contains(KeyModifiers::SUPER) {
        modifiers = modifiers | ConfigKeyModifiers::SUPER;
    }

    KeyBinding::new(code, modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_defaults() {
        let shortcuts = Shortcuts::load();
        assert_eq!(shortcuts.toggle_theme(), &KeyBinding::new(ConfigKeyCode::Char('t'), ConfigKeyModifiers::CONTROL));
        assert_eq!(shortcuts.exit(), &KeyBinding::new(ConfigKeyCode::Char('c'), ConfigKeyModifiers::CONTROL));
    }

    #[test]
    fn test_match_ctrl_t() {
        let shortcuts = Shortcuts::load();
        let event = KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        );
        assert!(shortcuts.matches(&event, shortcuts.toggle_theme()));
    }

    #[test]
    fn test_match_ctrl_l() {
        let shortcuts = Shortcuts::load();
        let event = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert!(shortcuts.matches(&event, shortcuts.clear_screen()));
    }

    #[test]
    fn test_match_shift_tab() {
        let shortcuts = Shortcuts::load();
        let event = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        assert!(shortcuts.matches(&event, shortcuts.cycle_agent_mode()));
    }

    #[test]
    fn test_no_match_wrong_modifier() {
        let shortcuts = Shortcuts::load();
        let event = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE);
        assert!(!shortcuts.matches(&event, shortcuts.toggle_theme()));
    }
}
