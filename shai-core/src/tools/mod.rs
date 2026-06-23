pub mod types;
pub mod highlight;
pub mod todo;
pub mod fs;
pub mod fetch;
pub mod bash;
pub mod mcp;
pub mod memory;
pub mod skills;

#[cfg(test)]
mod tests_llm;

pub use shai_macros::tool;
pub use types::{Tool, ToolCall, ToolResult, ToolError, ToolCapability, AnyTool, AnyToolBox, ToolEmptyParams};

// Re-export all tools
pub use bash::BashTool;
pub use fetch::FetchTool;
pub use fs::{EditTool, FindTool, LsTool, MultiEditTool, ReadTool, WriteTool, FsOperationLog, FsOperationType, FsOperation, FsOperationSummary};
pub use todo::{TodoReadTool, TodoWriteTool, TodoStorage, TodoItem, TodoStatus, TodoWriteParams, TodoItemInput};
pub use mcp::{McpClient, McpToolDescription, McpConfig, create_mcp_client, get_mcp_tools, StdioClient, HttpClient, SseClient};
