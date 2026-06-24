use super::super::{FsOperationLog, FsOperationType};
use super::structs::{ReadFileSpec, ReadToolParams};
use crate::tools::fs::hash::compute_line_hash;
use crate::tools::{tool, ToolResult};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct ReadTool {
    operation_log: Arc<FsOperationLog>,
}

impl ReadTool {
    pub fn new(operation_log: Arc<FsOperationLog>) -> Self {
        Self { operation_log }
    }

    fn read_file_content(&self, params: &ReadFileSpec) -> io::Result<String> {
        let file = fs::File::open(&params.path)?;
        let reader = BufReader::new(file);

        match (params.line_start, params.line_end) {
            // Read specific line range
            (Some(start), Some(end)) => {
                let lines: Result<Vec<(u32, String)>, io::Error> = reader
                    .lines()
                    .enumerate()
                    .filter_map(|(i, line)| {
                        let line_num = i as u32 + 1;
                        if line_num >= start && line_num <= end {
                            Some(line.map(|l| (line_num, l)))
                        } else {
                            None
                        }
                    })
                    .collect();

                match lines {
                    Ok(filtered_lines) => {
                        Ok(Self::format_lines(&filtered_lines, params.show_line_numbers))
                    }
                    Err(e) => Err(e),
                }
            }
            // Read from start line to end of file
            (Some(start), None) => {
                let lines: Result<Vec<(u32, String)>, io::Error> = reader
                    .lines()
                    .enumerate()
                    .filter_map(|(i, line)| {
                        let line_num = i as u32 + 1;
                        if line_num >= start {
                            Some(line.map(|l| (line_num, l)))
                        } else {
                            None
                        }
                    })
                    .collect();

                match lines {
                    Ok(filtered_lines) => {
                        Ok(Self::format_lines(&filtered_lines, params.show_line_numbers))
                    }
                    Err(e) => Err(e),
                }
            }
            // Read from beginning to end line
            (None, Some(end)) => {
                let lines: Result<Vec<(u32, String)>, io::Error> = reader
                    .lines()
                    .enumerate()
                    .filter_map(|(i, line)| {
                        let line_num = i as u32 + 1;
                        if line_num <= end {
                            Some(line.map(|l| (line_num, l)))
                        } else {
                            None
                        }
                    })
                    .collect();

                match lines {
                    Ok(filtered_lines) => {
                        Ok(Self::format_lines(&filtered_lines, params.show_line_numbers))
                    }
                    Err(e) => Err(e),
                }
            }
            // Read entire file
            (None, None) => {
                if params.show_line_numbers {
                    let lines: Result<Vec<(u32, String)>, io::Error> = reader
                        .lines()
                        .enumerate()
                        .map(|(i, line)| {
                            let line_num = i as u32 + 1;
                            line.map(|l| (line_num, l))
                        })
                        .collect();

                    match lines {
                        Ok(numbered_lines) => Ok(Self::format_lines(&numbered_lines, true)),
                        Err(e) => Err(e),
                    }
                } else {
                    fs::read_to_string(&params.path)
                }
            }
        }
    }

    fn format_lines(lines: &[(u32, String)], show_line_numbers: bool) -> String {
        if show_line_numbers {
            lines
                .iter()
                .map(|(line_num, content)| {
                    let hash = compute_line_hash(content);
                    format!("{:4}: {} {}", line_num, hash, content)
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            lines
                .iter()
                .map(|(_, content)| content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

#[tool(name = "read", description = r#"Reads one or more files in a single call. Each file's content is prefixed with its path.

**Usage:**
- Pass an array of file specs in the `files` parameter.
- Each spec requires a `path` and optionally `line_start`, `line_end`, and `show_line_numbers`.
- When `show_line_numbers` is true, output includes line numbers and hashes for each line.

**Best Practices:**
- When investigating a task, read multiple potentially relevant files in a single call to build context.
- Always use `show_line_numbers: true` when you plan to edit the files afterward — the line hashes enable precise targeting."#, capabilities = [Read])]
impl ReadTool {
    async fn execute(&self, params: ReadToolParams) -> ToolResult {
        if params.files.is_empty() {
            return ToolResult::error("At least one file path is required".to_string());
        }

        let mut outputs = Vec::new();
        let mut total_lines = 0usize;
        let mut file_count = 0usize;

        for file_spec in &params.files {
            let path = Path::new(&file_spec.path);

            if !path.exists() {
                outputs.push(format!("=== {} ===\n[Error: File does not exist]", file_spec.path));
                continue;
            }
            if !path.is_file() {
                outputs.push(format!("=== {} ===\n[Error: Path is not a file]", file_spec.path));
                continue;
            }

            match self.read_file_content(file_spec) {
                Ok(content) => {
                    total_lines += content.lines().count();
                    file_count += 1;
                    self.operation_log
                        .log_operation(FsOperationType::Read, file_spec.path.clone())
                        .await;
                    outputs.push(format!("=== {} ===\n{}", file_spec.path, content));
                }
                Err(e) => {
                    outputs.push(format!("=== {} ===\n[Error: {}]", file_spec.path, e));
                }
            }
        }

        let mut meta = HashMap::new();
        meta.insert("file_count".to_string(), json!(file_count));
        meta.insert("total_lines".to_string(), json!(total_lines));

        ToolResult::Success {
            output: outputs.join("\n\n"),
            metadata: Some(meta),
        }
    }
}
