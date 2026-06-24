use super::multiedit::{MultiEditTool, MultiFileEditTool};
use super::structs::{EditOperation, FileEdit, MultiEditToolParams, MultiFileEditToolParams};
use crate::tools::{FsOperationLog, Tool, ToolCapability};
use shai_llm::ToolDescription;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_multiedit_tool_permissions() {
    let log = Arc::new(FsOperationLog::new());
    let tool = MultiEditTool::new(log);
    let perms = tool.capabilities();
    assert!(perms.contains(&ToolCapability::Read));
    assert!(perms.contains(&ToolCapability::Write));
    assert_eq!(perms.len(), 2);
}

#[tokio::test]
async fn test_multiedit_tool_creation() {
    let log = Arc::new(FsOperationLog::new());
    let tool = MultiEditTool::new(log);
    assert_eq!(&tool.name(), "multiedit");
    assert!(!tool.description().is_empty());
}

#[tokio::test]
async fn test_multiedit_multiple_replacements() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "Hello World, Hello Universe").unwrap();

    let log = Arc::new(FsOperationLog::new());
    // First read the file to satisfy the logging requirement
    log.log_operation(
        crate::tools::FsOperationType::Read,
        file_path.to_string_lossy().to_string(),
    )
    .await;

    let tool = MultiEditTool::new(log);
    let params = MultiEditToolParams {
        file_path: file_path.to_string_lossy().to_string(),
        edits: vec![
            EditOperation {
                old_string: "Hello".to_string(),
                new_string: "Hi".to_string(),
                replace_all: false,
                line_hash: None,
            },
            EditOperation {
                old_string: "World".to_string(),
                new_string: "Earth".to_string(),
                replace_all: true,
                line_hash: None,
            },
        ],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_success());

    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "Hi Earth, Hello Universe");
}

#[tokio::test]
async fn test_multiedit_preview_diff_output() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "line1\nHello World\nline3\nGoodbye World").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(
        crate::tools::FsOperationType::Read,
        file_path.to_string_lossy().to_string(),
    )
    .await;

    let tool = MultiEditTool::new(log);
    let params = MultiEditToolParams {
        file_path: file_path.to_string_lossy().to_string(),
        edits: vec![
            EditOperation {
                old_string: "Hello".to_string(),
                new_string: "Hi".to_string(),
                replace_all: false,
                line_hash: None,
            },
            EditOperation {
                old_string: "Goodbye".to_string(),
                new_string: "Farewell".to_string(),
                replace_all: false,
                line_hash: None,
            },
        ],
    };

    // Test preview - should return Some(ToolResult) with diff
    let preview_result = tool.execute_preview(params.clone()).await;
    assert!(preview_result.is_some());

    let preview = preview_result.unwrap();
    assert!(preview.is_success());

    // Preview should contain diff output showing both changes
    let output = match preview {
        crate::tools::ToolResult::Success { output, .. } => output,
        _ => panic!("Expected success result"),
    };

    println!("MultiEdit diff output:\n{}", output);

    // Should contain diff markers for both changes
    assert!(output.contains("-")); // Deletion markers
    assert!(output.contains("+")); // Addition markers
    assert!(output.contains("Hello")); // First old content
    assert!(output.contains("Hi")); // First new content
    assert!(output.contains("Goodbye")); // Second old content
    assert!(output.contains("Farewell")); // Second new content

    // Should contain ANSI color codes
    assert!(output.contains("\x1b[48;5;88;37m")); // Red background for deletions
    assert!(output.contains("\x1b[48;5;28;37m")); // Green background for additions

    // Original file should be unchanged after preview
    let original_content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(original_content, "line1\nHello World\nline3\nGoodbye World");
}


// ===== MultiFileEditTool tests =====

#[tokio::test]
async fn test_multifileedit_basic() {
    let dir = tempdir().unwrap();
    let file_a = dir.path().join("a.txt");
    let file_b = dir.path().join("b.txt");
    fs::write(&file_a, "Hello World").unwrap();
    fs::write(&file_b, "Goodbye Moon").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(crate::tools::FsOperationType::Read, file_a.to_string_lossy().to_string()).await;
    log.log_operation(crate::tools::FsOperationType::Read, file_b.to_string_lossy().to_string()).await;

    let tool = MultiFileEditTool::new(log);
    let params = MultiFileEditToolParams {
        files: vec![
            FileEdit {
                file_path: file_a.to_string_lossy().to_string(),
                edits: vec![EditOperation {
                    old_string: "Hello".to_string(),
                    new_string: "Greetings".to_string(),
                    replace_all: false,
                    line_hash: None,
                }],
            },
            FileEdit {
                file_path: file_b.to_string_lossy().to_string(),
                edits: vec![EditOperation {
                    old_string: "Goodbye".to_string(),
                    new_string: "Farewell".to_string(),
                    replace_all: false,
                    line_hash: None,
                }],
            },
        ],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_success());
    assert_eq!(fs::read_to_string(&file_a).unwrap(), "Greetings World");
    assert_eq!(fs::read_to_string(&file_b).unwrap(), "Farewell Moon");
}

#[tokio::test]
async fn test_multifileedit_atomicity() {
    let dir = tempdir().unwrap();
    let file_a = dir.path().join("a.txt");
    let file_b = dir.path().join("b.txt");
    fs::write(&file_a, "Hello World").unwrap();
    fs::write(&file_b, "Goodbye Moon").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(crate::tools::FsOperationType::Read, file_a.to_string_lossy().to_string()).await;
    log.log_operation(crate::tools::FsOperationType::Read, file_b.to_string_lossy().to_string()).await;

    let tool = MultiFileEditTool::new(log);
    let params = MultiFileEditToolParams {
        files: vec![
            FileEdit {
                file_path: file_a.to_string_lossy().to_string(),
                edits: vec![EditOperation {
                    old_string: "Hello".to_string(),
                    new_string: "Hi".to_string(),
                    replace_all: false,
                    line_hash: None,
                }],
            },
            FileEdit {
                file_path: file_b.to_string_lossy().to_string(),
                edits: vec![EditOperation {
                    old_string: "NONEXISTENT".to_string(),
                    new_string: "error".to_string(),
                    replace_all: false,
                    line_hash: None,
                }],
            },
        ],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_error());
    // First file should NOT be modified because second edit failed
    assert_eq!(fs::read_to_string(&file_a).unwrap(), "Hello World");
    assert_eq!(fs::read_to_string(&file_b).unwrap(), "Goodbye Moon");
}

#[tokio::test]
async fn test_multifileedit_not_read_yet() {
    let dir = tempdir().unwrap();
    let file_a = dir.path().join("a.txt");
    fs::write(&file_a, "Hello World").unwrap();

    let log = Arc::new(FsOperationLog::new());
    let tool = MultiFileEditTool::new(log);
    let params = MultiFileEditToolParams {
        files: vec![FileEdit {
            file_path: file_a.to_string_lossy().to_string(),
            edits: vec![EditOperation {
                old_string: "Hello".to_string(),
                new_string: "Hi".to_string(),
                replace_all: false,
                line_hash: None,
            }],
        }],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_error());
}

#[tokio::test]
async fn test_multifileedit_empty_params() {
    let log = Arc::new(FsOperationLog::new());
    let tool = MultiFileEditTool::new(log);
    let params = MultiFileEditToolParams {
        files: vec![],
    };
    let result = tool.execute(params, None).await;
    assert!(result.is_error());
}

#[tokio::test]
async fn test_drain_edited_files() {
    let log = FsOperationLog::new();
    log.log_operation(crate::tools::FsOperationType::Edit, "foo.rs".to_string()).await;
    log.log_operation(crate::tools::FsOperationType::MultiEdit, "bar.py".to_string()).await;
    log.log_operation(crate::tools::FsOperationType::Read, "baz.txt".to_string()).await;

    let drained = log.drain_edited_files().await;
    assert_eq!(drained.len(), 2);
    assert!(drained.contains(&"foo.rs".to_string()));
    assert!(drained.contains(&"bar.py".to_string()));

    // Second drain should be empty
    let drained_again = log.drain_edited_files().await;
    assert!(drained_again.is_empty());
}

#[tokio::test]
async fn test_verification_config_default() {
    let config = crate::config::agent::VerificationConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.timeout_secs, 30);
    assert!(config.commands.contains_key("rust"));
    assert!(config.commands.contains_key("python"));
    assert!(config.commands.contains_key("go"));
}

// ===== Hash-anchored edit tests =====

#[tokio::test]
async fn test_hash_anchored_edit_basic() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "fn hello() {\n    println!(\"hello\");\n}").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(
        crate::tools::FsOperationType::Read,
        file_path.to_string_lossy().to_string(),
    )
    .await;

    let tool = MultiEditTool::new(log);
    let params = MultiEditToolParams {
        file_path: file_path.to_string_lossy().to_string(),
        edits: vec![EditOperation {
            old_string: String::new(),
            new_string: "    println!(\"goodbye\");".to_string(),
            replace_all: false,
            line_hash: Some(crate::tools::fs::hash::compute_line_hash("    println!(\"hello\");")),
        }],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_success());
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("goodbye"));
    assert!(!content.contains("hello\");"));
}

#[tokio::test]
async fn test_hash_anchored_edit_not_found() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.rs");
    fs::write(&file_path, "fn hello() {\n}\n").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(
        crate::tools::FsOperationType::Read,
        file_path.to_string_lossy().to_string(),
    )
    .await;

    let tool = MultiEditTool::new(log);
    let params = MultiEditToolParams {
        file_path: file_path.to_string_lossy().to_string(),
        edits: vec![EditOperation {
            old_string: String::new(),
            new_string: "replaced".to_string(),
            replace_all: false,
            line_hash: Some("deadbeef".to_string()),
        }],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_error());
}

#[tokio::test]
async fn test_hash_anchored_edit_replace_all() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.py");
    fs::write(&file_path, "x = 1\nx = 1\nx = 1").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(
        crate::tools::FsOperationType::Read,
        file_path.to_string_lossy().to_string(),
    )
    .await;

    let tool = MultiEditTool::new(log);
    let params = MultiEditToolParams {
        file_path: file_path.to_string_lossy().to_string(),
        edits: vec![EditOperation {
            old_string: String::new(),
            new_string: "x = 0".to_string(),
            replace_all: true,
            line_hash: Some(crate::tools::fs::hash::compute_line_hash("x = 1")),
        }],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_success());
    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "x = 0\nx = 0\nx = 0");
}

#[tokio::test]
async fn test_hash_anchored_edit_first_match_only() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.py");
    fs::write(&file_path, "x = 1\nx = 1\nx = 1").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(
        crate::tools::FsOperationType::Read,
        file_path.to_string_lossy().to_string(),
    )
    .await;

    let tool = MultiEditTool::new(log);
    let params = MultiEditToolParams {
        file_path: file_path.to_string_lossy().to_string(),
        edits: vec![EditOperation {
            old_string: String::new(),
            new_string: "x = 0".to_string(),
            replace_all: false,
            line_hash: Some(crate::tools::fs::hash::compute_line_hash("x = 1")),
        }],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_success());
    let content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "x = 0\nx = 1\nx = 1");
}

#[tokio::test]
async fn test_hash_anchored_multifile_edit() {
    let dir = tempdir().unwrap();
    let file_a = dir.path().join("a.rs");
    let file_b = dir.path().join("b.rs");
    fs::write(&file_a, "fn foo() {}\n").unwrap();
    fs::write(&file_b, "fn bar() {}\n").unwrap();

    let log = Arc::new(FsOperationLog::new());
    log.log_operation(crate::tools::FsOperationType::Read, file_a.to_string_lossy().to_string()).await;
    log.log_operation(crate::tools::FsOperationType::Read, file_b.to_string_lossy().to_string()).await;

    let tool = MultiFileEditTool::new(log);
    let params = MultiFileEditToolParams {
        files: vec![
            FileEdit {
                file_path: file_a.to_string_lossy().to_string(),
                edits: vec![EditOperation {
                    old_string: String::new(),
                    new_string: "fn foo() -> u32 { 42 }".to_string(),
                    replace_all: false,
                    line_hash: Some(crate::tools::fs::hash::compute_line_hash("fn foo() {}")),
                }],
            },
            FileEdit {
                file_path: file_b.to_string_lossy().to_string(),
                edits: vec![EditOperation {
                    old_string: String::new(),
                    new_string: "fn bar() -> u32 { 0 }".to_string(),
                    replace_all: false,
                    line_hash: Some(crate::tools::fs::hash::compute_line_hash("fn bar() {}")),
                }],
            },
        ],
    };

    let result = tool.execute(params, None).await;
    assert!(result.is_success());
    assert_eq!(fs::read_to_string(&file_a).unwrap(), "fn foo() -> u32 { 42 }");
    assert_eq!(fs::read_to_string(&file_b).unwrap(), "fn bar() -> u32 { 0 }");
}

#[tokio::test]
async fn test_compute_line_hash_consistency() {
    use crate::tools::fs::hash::compute_line_hash;
    assert_eq!(compute_line_hash("hello"), compute_line_hash("hello"));
    assert_ne!(compute_line_hash("hello"), compute_line_hash("world"));
    assert_eq!(compute_line_hash("").len(), 8);
}
