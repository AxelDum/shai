use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct EditOperation {
    /// The text pattern to find and replace. Required unless `line_hash` is provided.
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

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MultiEditToolParams {
    /// Path to the file to edit
    pub file_path: String,
    /// Array of edit operations to perform sequentially
    pub edits: Vec<EditOperation>,
}

/// A single file's worth of edits for multi-file editing.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FileEdit {
    /// Path to the file to edit
    pub file_path: String,
    /// Array of edit operations to perform sequentially on this file
    pub edits: Vec<EditOperation>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MultiFileEditToolParams {
    /// Array of files, each with their own edit operations.
    /// All edits are applied atomically — if any edit fails, no files are modified.
    pub files: Vec<FileEdit>,
}
