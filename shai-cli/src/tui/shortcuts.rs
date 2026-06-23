/// Default keyboard shortcuts for the TUI.
///
/// This module provides a configurable shortcut system. Users can override
/// defaults by setting environment variables prefixed with `SHAI_KEY_`.
///
/// Currently supported shortcuts:
///   - `SHAI_KEY_TOGGLE_THEME`   — toggle dark/light theme (default: Ctrl+T)
///   - `SHAI_KEY_CANCEL_TASK`    — cancel running agent task (default: Esc)
///   - `SHAI_KEY_CLEAR_INPUT`    — clear input buffer (default: Esc x2)
///   - `SHAI_KEY_PASTE`         — paste from clipboard (default: Ctrl+V)
///   - `SHAI_KEY_EXIT`          — exit the TUI (default: Ctrl+C / Ctrl+D)
pub struct Shortcuts {
    pub toggle_theme: String,
    pub cancel_task: String,
    pub clear_input: String,
    pub paste: String,
    pub exit: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            toggle_theme: "ctrl+t".to_string(),
            cancel_task: "esc".to_string(),
            clear_input: "esc".to_string(),
            paste: "ctrl+v".to_string(),
            exit: "ctrl+c".to_string(),
        }
    }
}

impl Shortcuts {
    /// Load shortcuts from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            toggle_theme: std::env::var("SHAI_KEY_TOGGLE_THEME")
                .unwrap_or(defaults.toggle_theme),
            cancel_task: std::env::var("SHAI_KEY_CANCEL_TASK")
                .unwrap_or(defaults.cancel_task),
            clear_input: std::env::var("SHAI_KEY_CLEAR_INPUT")
                .unwrap_or(defaults.clear_input),
            paste: std::env::var("SHAI_KEY_PASTE")
                .unwrap_or(defaults.paste),
            exit: std::env::var("SHAI_KEY_EXIT")
                .unwrap_or(defaults.exit),
        }
    }

    /// Return a human-readable list of all configured shortcuts.
    pub fn descriptions(&self) -> Vec<(String, String)> {
        vec![
            ("toggle theme".to_string(), self.toggle_theme.clone()),
            ("cancel task".to_string(), self.cancel_task.clone()),
            ("clear input".to_string(), self.clear_input.clone()),
            ("paste".to_string(), self.paste.clone()),
            ("exit".to_string(), self.exit.clone()),
        ]
    }
}
