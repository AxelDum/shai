pub mod bash;
pub mod fetch;
pub mod fs;
pub mod highlight;
pub mod mcp;
pub mod memory;
pub mod skills;
pub mod todo;
pub mod types;

#[cfg(test)]
mod tests_llm;

pub use shai_macros::tool;
pub use types::{
    AnyTool, AnyToolBox, Tool, ToolCall, ToolCapability, ToolEmptyParams, ToolError, ToolResult,
};

// Re-export all tools
pub use bash::BashTool;
pub use fetch::FetchTool;
pub use fs::{
    EditTool, FindTool, FsOperation, FsOperationLog, FsOperationSummary, FsOperationType, LsTool,
    MultiEditTool, MultiFileEditTool, ReadTool, WriteTool,
};
pub use mcp::{
    create_mcp_client, get_mcp_tools, HttpClient, McpClient, McpConfig, McpToolDescription,
    SseClient, StdioClient,
};
pub use todo::{
    TodoItem, TodoItemInput, TodoReadTool, TodoStatus, TodoStorage, TodoWriteParams, TodoWriteTool,
};
