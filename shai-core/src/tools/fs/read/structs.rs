use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadFileSpec {
    /// Path to the file to read
    pub path: String,
    /// Starting line number (optional)
    #[serde(default)]
    pub line_start: Option<u32>,
    /// Ending line number (optional)
    #[serde(default)]
    pub line_end: Option<u32>,
    /// Whether to include line numbers and hashes in the output
    #[serde(default)]
    pub show_line_numbers: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadToolParams {
    /// One or more files to read
    pub files: Vec<ReadFileSpec>,
}
