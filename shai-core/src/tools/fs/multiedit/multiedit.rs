use super::super::{EditTool, FsOperationLog, FsOperationType};
use super::structs::{MultiEditToolParams, MultiFileEditToolParams};
use crate::tools::{tool, ToolResult};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct MultiEditTool {
    operation_log: Arc<FsOperationLog>,
    edit_tool: EditTool,
}

impl MultiEditTool {
    pub fn new(operation_log: Arc<FsOperationLog>) -> Self {
        Self::with_context_lines(operation_log, 3)
    }

    pub fn with_context_lines(operation_log: Arc<FsOperationLog>, context_lines: usize) -> Self {
        let edit_tool = EditTool::with_context_lines(operation_log.clone(), context_lines);
        Self {
            operation_log,
            edit_tool,
        }
    }

    async fn perform_multi_edit(
        &self,
        params: &MultiEditToolParams,
        preview: bool,
    ) -> Result<(String, Vec<usize>), String> {
        let path = Path::new(&params.file_path);

        // Check if file exists
        if !path.exists() {
            return Err(format!("File does not exist: {}", params.file_path));
        }

        // Read initial content
        let mut current_content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let original_content = current_content.clone();
        let mut replacements_per_edit = Vec::new();

        // Apply each edit operation sequentially on content
        for (index, edit) in params.edits.iter().enumerate() {
            match self.edit_tool.perform_edit_on_content(
                &current_content,
                &edit.old_string,
                &edit.new_string,
                edit.replace_all,
                edit.line_hash.as_deref(),
            ) {
                Ok((new_content, replacements)) => {
                    current_content = new_content;
                    replacements_per_edit.push(replacements);
                }
                Err(error) => {
                    return Err(format!("Edit #{}: {}", index + 1, error));
                }
            }
        }

        // Generate comprehensive diff
        let diff = self
            .edit_tool
            .myers_diff(&original_content, &current_content);

        // Only write to file if not preview mode
        if !preview {
            self.edit_tool
                .commit_edit(&params.file_path, &current_content)?;
        }

        Ok((diff, replacements_per_edit))
    }
}

#[tool(name = "multiedit", description = r#"Executes a batch of sequential find-and-replace operations on a single file within one atomic transaction. This is the preferred tool for making numerous, distinct changes to one file efficiently.

**Execution Logic:**
- Edits are applied in the exact order they are provided. The second edit operates on the result of the first, the third on the result of the second, and so on.
- The entire sequence is atomic. If any single edit fails (e.g., its `old_string` is not found), the whole operation is rolled back, and the file remains unmodified.

**Critical Considerations:**
- You must first use the `read` tool to understand the file's contents.
- Plan your sequence of edits carefully. An earlier edit might alter the text that a later edit is intended to match, which could cause the later edit to fail."#, capabilities = [ToolCapability::Read, ToolCapability::Write])]
impl MultiEditTool {
    async fn execute_preview(&self, params: MultiEditToolParams) -> Option<ToolResult> {
        Some(self.execute_internal(params, true).await)
    }

    async fn execute(&self, params: MultiEditToolParams) -> ToolResult {
        self.execute_internal(params, false).await
    }

    async fn execute_internal(&self, params: MultiEditToolParams, preview: bool) -> ToolResult {
        // Validate that we have at least one edit operation
        if params.edits.is_empty() {
            return ToolResult::error("At least one edit operation is required".to_string());
        }

        // Validate that the file has been read first
        if let Err(err) = self
            .operation_log
            .validate_edit_permission(&params.file_path)
            .await
        {
            return ToolResult::error(err);
        }

        match self.perform_multi_edit(&params, preview).await {
            Ok((message, replacements_per_edit)) => {
                // Log the multiedit operation only if not preview
                if !preview {
                    self.operation_log
                        .log_operation(FsOperationType::MultiEdit, params.file_path.clone())
                        .await;
                }

                let mut meta = HashMap::new();
                meta.insert("path".to_string(), json!(params.file_path));
                meta.insert("edit_count".to_string(), json!(params.edits.len()));
                meta.insert(
                    "total_replacements".to_string(),
                    json!(replacements_per_edit.iter().sum::<usize>()),
                );
                meta.insert(
                    "replacements_per_edit".to_string(),
                    json!(replacements_per_edit),
                );
                meta.insert("preview_mode".to_string(), json!(preview));

                // Add detailed information about each edit
                let edit_details: Vec<serde_json::Value> = params
                    .edits
                    .iter()
                    .enumerate()
                    .map(|(i, edit)| {
                        json!({
                            "index": i,
                            "old_string": edit.old_string,
                            "new_string": edit.new_string,
                            "replace_all": edit.replace_all,
                            "replacements_made": replacements_per_edit[i]
                        })
                    })
                    .collect();
                meta.insert("edit_details".to_string(), json!(edit_details));

                // Add file size information
                if let Ok(metadata) = std::fs::metadata(&params.file_path) {
                    meta.insert("file_size_bytes".to_string(), json!(metadata.len()));
                }

                ToolResult::Success {
                    output: message,
                    metadata: Some(meta),
                }
            }
            Err(e) => ToolResult::error(format!(
                "MultiEdit {} failed: {}",
                if preview { "preview" } else { "" },
                e
            )),
        }
    }
}

#[derive(Clone)]
pub struct MultiFileEditTool {
    operation_log: Arc<FsOperationLog>,
    edit_tool: EditTool,
}

impl MultiFileEditTool {
    pub fn new(operation_log: Arc<FsOperationLog>) -> Self {
        Self::with_context_lines(operation_log, 3)
    }

    pub fn with_context_lines(operation_log: Arc<FsOperationLog>, context_lines: usize) -> Self {
        let edit_tool = EditTool::with_context_lines(operation_log.clone(), context_lines);
        Self {
            operation_log,
            edit_tool,
        }
    }

    async fn perform_multi_file_edit(
        &self,
        params: &MultiFileEditToolParams,
        preview: bool,
    ) -> Result<String, String> {
        // Phase 1: Validate all files have been read
        for file_edit in &params.files {
            self.operation_log
                .validate_edit_permission(&file_edit.file_path)
                .await?;
        }

        // Phase 2: Apply all edits in memory
        let mut staged_writes: Vec<(String, String)> = Vec::new();
        let mut diffs = Vec::new();

        for file_edit in &params.files {
            let path = Path::new(&file_edit.file_path);
            if !path.exists() {
                return Err(format!("File does not exist: {}", file_edit.file_path));
            }

            let mut current_content =
                fs::read_to_string(path).map_err(|e| e.to_string())?;
            let original_content = current_content.clone();

            for (index, edit) in file_edit.edits.iter().enumerate() {
                match self.edit_tool.perform_edit_on_content(
                    &current_content,
                    &edit.old_string,
                    &edit.new_string,
                    edit.replace_all,
                    edit.line_hash.as_deref(),
                ) {
                    Ok((new_content, _replacements)) => {
                        current_content = new_content;
                    }
                    Err(error) => {
                        return Err(format!(
                            "File '{}', Edit #{}: {}",
                            file_edit.file_path,
                            index + 1,
                            error
                        ));
                    }
                }
            }

            let diff = self.edit_tool.myers_diff(&original_content, &current_content);
            diffs.push(diff);
            staged_writes.push((file_edit.file_path.clone(), current_content));
        }

        // Phase 3: Write all files (only after all edits succeeded)
        if !preview {
            for (file_path, content) in &staged_writes {
                self.edit_tool.commit_edit(file_path, content)?;
            }
        }

        // Phase 4: Log all operations
        if !preview {
            for file_edit in &params.files {
                self.operation_log
                    .log_operation(FsOperationType::MultiEdit, file_edit.file_path.clone())
                    .await;
            }
        }

        let combined_diff = diffs.join("\n\n");
        Ok(combined_diff)
    }
}

#[tool(name = "multifileedit", description = r#"Executes find-and-replace operations across multiple files in one atomic transaction. All edits are applied in memory first; if any edit fails, no files are modified.

**Parameters:**
- `files`: Array of `{ file_path: string, edits: [{ old_string: string, new_string: string, replace_all?: bool }] }`

**Critical:**
- Every file must have been read first using the `read` tool.
- Edits within each file are applied sequentially.
- The entire operation is atomic — if any edit fails, no files are modified."#, capabilities = [ToolCapability::Read, ToolCapability::Write])]
impl MultiFileEditTool {
    async fn execute_preview(&self, params: MultiFileEditToolParams) -> Option<ToolResult> {
        Some(self.execute_internal(params, true).await)
    }

    async fn execute(&self, params: MultiFileEditToolParams) -> ToolResult {
        self.execute_internal(params, false).await
    }

    async fn execute_internal(&self, params: MultiFileEditToolParams, preview: bool) -> ToolResult {
        if params.files.is_empty() {
            return ToolResult::error("At least one file edit is required".to_string());
        }

        match self.perform_multi_file_edit(&params, preview).await {
            Ok(diff) => {
                let mut meta = HashMap::new();
                meta.insert("file_count".to_string(), json!(params.files.len()));
                meta.insert(
                    "total_edits".to_string(),
                    json!(params.files.iter().map(|f| f.edits.len()).sum::<usize>()),
                );
                meta.insert("preview_mode".to_string(), json!(preview));

                let file_details: Vec<serde_json::Value> = params
                    .files
                    .iter()
                    .map(|f| {
                        json!({
                            "file_path": f.file_path,
                            "edit_count": f.edits.len(),
                        })
                    })
                    .collect();
                meta.insert("files".to_string(), json!(file_details));

                ToolResult::Success {
                    output: diff,
                    metadata: Some(meta),
                }
            }
            Err(e) => ToolResult::error(format!(
                "MultiFileEdit {} failed: {}",
                if preview { "preview" } else { "" },
                e
            )),
        }
    }
}
