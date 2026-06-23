use super::super::{FsOperationLog, FsOperationType};
use super::structs::EditToolParams;
use crate::tools::{tool, ToolResult};
use serde_json::json;
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct EditTool {
    operation_log: Arc<FsOperationLog>,
    context_lines: usize,
}

impl EditTool {
    pub fn new(operation_log: Arc<FsOperationLog>) -> Self {
        Self::with_context_lines(operation_log, 3)
    }

    pub fn with_context_lines(operation_log: Arc<FsOperationLog>, context_lines: usize) -> Self {
        Self {
            operation_log,
            context_lines,
        }
    }

    pub fn myers_diff(&self, before_content: &str, after_content: &str) -> String {
        let diff = TextDiff::from_lines(before_content, after_content);

        // Check if there are any changes
        let has_changes = diff
            .iter_all_changes()
            .any(|change| change.tag() != ChangeTag::Equal);
        if !has_changes {
            return "No changes".to_string();
        }

        // Collect all changes with their positions and metadata
        #[derive(Debug)]
        struct ChangeInfo {
            tag: ChangeTag,
            content: String,
            old_line: usize,
            new_line: usize,
        }

        let mut all_changes = Vec::new();
        let mut line_num_old = 1;
        let mut line_num_new = 1;

        // Collect all changes first
        for change in diff.iter_all_changes() {
            let change_info = ChangeInfo {
                tag: change.tag(),
                content: change.value().trim_end().to_string(),
                old_line: line_num_old,
                new_line: line_num_new,
            };

            all_changes.push(change_info);

            // Update line numbers
            match change.tag() {
                ChangeTag::Delete => line_num_old += 1,
                ChangeTag::Insert => line_num_new += 1,
                ChangeTag::Equal => {
                    line_num_old += 1;
                    line_num_new += 1;
                }
            }
        }

        // Find positions of actual changes (not Equal)
        let mut change_positions = Vec::new();
        for (idx, change) in all_changes.iter().enumerate() {
            if change.tag != ChangeTag::Equal {
                change_positions.push(idx);
            }
        }

        if change_positions.is_empty() {
            return "No changes".to_string();
        }

        // Calculate ranges to show (changes + context)
        let mut ranges_to_show = Vec::new();
        let context = self.context_lines;

        let mut current_start = change_positions[0].saturating_sub(context);
        let mut current_end = (change_positions[0] + context + 1).min(all_changes.len());

        // Merge overlapping ranges
        for &pos in &change_positions[1..] {
            let range_start = pos.saturating_sub(context);
            let range_end = (pos + context + 1).min(all_changes.len());

            // If ranges overlap or are adjacent, merge them
            if range_start <= current_end {
                current_end = range_end;
            } else {
                // Save current range and start new one
                ranges_to_show.push((current_start, current_end));
                current_start = range_start;
                current_end = range_end;
            }
        }
        ranges_to_show.push((current_start, current_end));

        // Generate focused diff output
        let mut diff_output = Vec::new();

        for (range_idx, &(start, end)) in ranges_to_show.iter().enumerate() {
            // Add separator between ranges (if there are multiple ranges)
            if range_idx > 0 {
                diff_output.push("\x1b[2;37m...\x1b[0m".to_string());
            }

            // Show lines in this range
            for change in &all_changes[start..end] {
                let (sign, style) = match change.tag {
                    ChangeTag::Delete => ("-", "\x1b[48;5;88;37m"), // Dark red background
                    ChangeTag::Insert => ("+", "\x1b[48;5;28;37m"), // Dark green background
                    ChangeTag::Equal => (" ", ""),
                };

                let line_no = match change.tag {
                    ChangeTag::Delete => change.old_line,
                    ChangeTag::Insert => change.new_line,
                    ChangeTag::Equal => change.old_line,
                };

                if change.tag == ChangeTag::Equal {
                    // Context line
                    diff_output.push(format!(
                        "\x1b[2;37m{:4}\x1b[0m   {}",
                        line_no, change.content
                    ));
                } else {
                    // Changed line
                    diff_output.push(format!(
                        "\x1b[2;37m{:4}\x1b[0m {}{} {}\x1b[0m",
                        line_no, style, sign, change.content
                    ));
                }
            }
        }

        diff_output.join("\n")
    }

    pub fn perform_edit_on_content(
        &self,
        content: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<(String, usize), String> {
        // Check if the old_string exists in the content
        if !content.contains(old_string) {
            return Err("Pattern not found in file".to_string());
        }

        // Perform the replacement
        let (new_content, replacements) = if replace_all {
            let new_content = content.replace(old_string, new_string);
            let replacements = content.matches(old_string).count();
            (new_content, replacements)
        } else {
            let new_content = content.replacen(old_string, new_string, 1);
            (new_content, 1)
        };

        Ok((new_content, replacements))
    }

    pub fn commit_edit(&self, path: &str, new_content: &str) -> Result<(), String> {
        fs::write(path, new_content).map_err(|e| e.to_string())
    }

    fn perform_edit(
        &self,
        params: &EditToolParams,
        preview: bool,
    ) -> Result<(String, usize), String> {
        let path = Path::new(&params.path);

        // Check if file exists
        if !path.exists() {
            return Err(format!("File does not exist: {}", params.path));
        }

        // Read the file content
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;

        // Perform edit on content
        let (new_content, replacements) = self.perform_edit_on_content(
            &content,
            &params.old_string,
            &params.new_string,
            params.replace_all,
        )?;

        // Generate proper diff using Myers' algorithm
        let diff = self.myers_diff(&content, &new_content);

        let mut diff_output = vec!["".to_string()];
        diff_output.push(diff);

        // Only write to file if not preview mode
        if !preview {
            self.commit_edit(&params.path, &new_content)?;
        }

        Ok((diff_output.join("\n"), replacements))
    }
}

#[tool(name = "edit", description = r#"Facilitates targeted modifications within a file by replacing a specific segment of text. This tool is for surgical precision.

**Prerequisites:**
- Before using this tool, you are required to have inspected the file's content using the `read` tool in the current conversation. An attempt to edit a file without prior reading will result in an error.

**Usage Guidelines:**
- The `old_string` parameter demands an exact, literal match of the text to be replaced. This includes all whitespace and indentation. When copying text from the `read` tool's output, you must omit the line number prefix.
- The operation will fail if the `old_string` is not unique within the file. To resolve this, provide more surrounding context to make the `old_string` unique.
- For situations where you intend to replace every occurrence of a string (e.g., renaming a variable), set the `replace_all` parameter to `true`.
- Prioritize modifying existing files. Avoid creating new files unless the task explicitly requires it.
"#, capabilities = [ToolCapability::Read, ToolCapability::Write])]
impl EditTool {
    async fn execute_preview(&self, params: EditToolParams) -> Option<ToolResult> {
        Some(self.execute_internal(params, true).await)
    }

    async fn execute(&self, params: EditToolParams) -> ToolResult {
        self.execute_internal(params, false).await
    }

    async fn execute_internal(&self, params: EditToolParams, preview: bool) -> ToolResult {
        // Validate that old_string and new_string are different
        if params.old_string == params.new_string {
            return ToolResult::error("old_string and new_string cannot be the same".to_string());
        }

        // Validate that the file has been read first
        if let Err(err) = self
            .operation_log
            .validate_edit_permission(&params.path)
            .await
        {
            return ToolResult::error(err);
        }

        match self.perform_edit(&params, preview) {
            Ok((message, replacement_count)) => {
                // Log the edit operation only if not preview
                if !preview {
                    self.operation_log
                        .log_operation(FsOperationType::Edit, params.path.clone())
                        .await;
                }

                let mut meta = HashMap::new();
                meta.insert("path".to_string(), json!(params.path));
                meta.insert("old_string".to_string(), json!(params.old_string));
                meta.insert("new_string".to_string(), json!(params.new_string));
                meta.insert("replace_all".to_string(), json!(params.replace_all));
                meta.insert("replacements_made".to_string(), json!(replacement_count));
                meta.insert("preview_mode".to_string(), json!(preview));

                // Add file size information
                if let Ok(metadata) = std::fs::metadata(&params.path) {
                    meta.insert("file_size_bytes".to_string(), json!(metadata.len()));
                }

                ToolResult::Success {
                    output: message,
                    metadata: Some(meta),
                }
            }
            Err(e) => ToolResult::error(format!(
                "Edit {} failed: {}",
                if preview { "preview" } else { "" },
                e
            )),
        }
    }
}
