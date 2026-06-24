use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EditToolParams {
    /// Path to the file to edit
    pub path: String,
    /// The text pattern to find and replace. Required unless `line_hash` is provided.
    #[serde(default)]
    pub old_string: String,
    /// The replacement text
    pub new_string: String,
    /// Whether to replace all occurrences (default: false, replaces only first)
    #[serde(default)]
    pub replace_all: bool,
    /// Optional hash anchor — when provided, replaces the line(s) matching this hash
    /// with `new_string` instead of using `old_string` matching.
    #[serde(default)]
    pub line_hash: Option<String>,
}
