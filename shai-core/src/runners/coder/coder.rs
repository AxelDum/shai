use std::sync::Arc;

use async_trait::async_trait;
use openai_dive::v1::resources::chat::{
    ChatCompletionParametersBuilder, ChatMessage, ChatMessageContent,
};
use shai_llm::client::LlmClient;
use tracing::debug;

use crate::agent::brain::ThinkerDecision;
use crate::agent::{AgentBuilder, AgentCore, AgentError, Brain, ThinkerContext};
use crate::tools::skills::SkillTool;
use crate::tools::types::{ContainsAnyTool, IntoToolBox};
use crate::tools::{
    AnyTool, BashTool, EditTool, FetchTool, FindTool, FsOperationLog, LsTool, ReadTool,
    TodoReadTool, TodoStorage, TodoWriteTool, WriteTool,
};
use shai_llm::tool::LlmToolCall;

use super::prompt::{get_todo_read, render_system_prompt_template};
use crate::runners::compacter::compact_trace_if_needed;

#[derive(Clone)]
pub struct CoderBrain {
    pub llm: Arc<LlmClient>,
    pub model: String,
    pub system_prompt_template: String,
    pub temperature: f32,
    pub cached_prompt: Option<String>,
}

impl CoderBrain {
    pub fn new(llm: Arc<LlmClient>, model: String) -> Self {
        debug!(target: "brain::coder", provider =?llm.provider_name(), model = ?model);
        Self {
            llm,
            model,
            system_prompt_template: "{{CODER_BASE_PROMPT}}".to_string(),
            temperature: 0.0,
            cached_prompt: None,
        }
    }

    pub fn with_custom_prompt(
        llm: Arc<LlmClient>,
        model: String,
        system_prompt_template: String,
        temperature: f32,
    ) -> Self {
        debug!(target: "brain::coder", provider =?llm.provider_name(), model = ?model);
        Self {
            llm,
            model,
            system_prompt_template,
            temperature,
            cached_prompt: None,
        }
    }
}

#[async_trait]
impl Brain for CoderBrain {
    async fn next_step(&mut self, context: ThinkerContext) -> Result<ThinkerDecision, AgentError> {
        let mut trace = context.trace.read().await.clone();

        // Apply session-level trace compaction if needed
        if context.max_trace_chars > 0 {
            compact_trace_if_needed(&mut trace, context.max_trace_chars);
        }

        // Render the user's system prompt template (cached after first call)
        let system_prompt = match &self.cached_prompt {
            Some(cached) => cached.clone(),
            None => {
                let rendered = render_system_prompt_template(&self.system_prompt_template);
                self.cached_prompt = Some(rendered.clone());
                rendered
            }
        };

        // Add todo status if available
        let mut system_prompt_full = system_prompt;
        if let Some(tool) = context.available_tools.get_tool("todo_read") {
            let todo_status = get_todo_read(&tool).await;
            system_prompt_full += &todo_status;
        }

        trace.insert(
            0,
            ChatMessage::System {
                content: ChatMessageContent::Text(system_prompt_full),
                name: None,
            },
        );

        // get next step with custom temperature
        debug!(target: "brain::coder", temperature = context.temperature, "temperature");
        let request = ChatCompletionParametersBuilder::default()
            .model(&self.model)
            .messages(trace)
            .temperature(context.temperature)
            .build()
            .map_err(|e| AgentError::LlmError(e.to_string()))?;

        let brain_decision = self
            .llm
            .chat_with_tools(
                request,
                &context.available_tools.into_toolbox(),
                context.method,
            )
            .await
            .map_err(|e| AgentError::LlmError(e.to_string()))?;

        // Extract token usage information
        let token_usage = brain_decision.usage.as_ref().map(|usage| {
            let input = usage.prompt_tokens.unwrap_or(0);
            let output = usage.completion_tokens.unwrap_or(0);
            let cached = usage
                .input_tokens_details
                .as_ref()
                .map(|d| d.cached_tokens)
                .or_else(|| {
                    usage
                        .prompt_tokens_details
                        .as_ref()
                        .map(|d| d.cached_tokens)
                })
                .unwrap_or(0);
            (input, output, cached)
        });

        debug!(target: "brain::coder", usage = ?brain_decision.usage, "LLM response usage");

        // stop here if there's no other tool calls
        let message = brain_decision.choices.into_iter().next().unwrap().message;
        if let ChatMessage::Assistant { tool_calls, .. } = &message {
            if tool_calls.as_ref().map_or(true, |calls| calls.is_empty()) {
                return Ok(match token_usage {
                    Some((input_tokens, output_tokens, cached_tokens)) => {
                        ThinkerDecision::agent_pause_with_tokens(
                            message,
                            input_tokens,
                            output_tokens,
                            cached_tokens,
                        )
                    }
                    None => ThinkerDecision::agent_pause(message),
                });
            }
        }
        Ok(match token_usage {
            Some((input_tokens, output_tokens, cached_tokens)) => {
                ThinkerDecision::agent_continue_with_tokens(
                    message,
                    input_tokens,
                    output_tokens,
                    cached_tokens,
                )
            }
            None => ThinkerDecision::agent_continue(message),
        })
    }
}

/// Coder agent factory — returns AgentCore directly so callers can configure working_dir.
pub fn coder(llm: Arc<LlmClient>, model: String) -> AgentCore {
    // Create shared storage for todo tools
    let todo_storage = Arc::new(TodoStorage::new());

    // Create shared operation log for file system tools
    let fs_log = Arc::new(FsOperationLog::new());

    let bash = Box::new(BashTool::new());
    let edit = Box::new(EditTool::new(fs_log.clone()));
    let fetch = Box::new(FetchTool::new());
    let find = Box::new(FindTool::new());
    let ls = Box::new(LsTool::new());
    let read = Box::new(ReadTool::new(fs_log.clone()));
    let todoread = Box::new(TodoReadTool::new(todo_storage.clone()));
    let todowrite = Box::new(TodoWriteTool::new(todo_storage.clone()));
    let write = Box::new(WriteTool::new(fs_log.clone()));
    let toolbox: Vec<Box<dyn AnyTool>> = vec![
        bash,
        edit,
        fetch,
        find,
        ls,
        read,
        todoread,
        todowrite,
        write,
        Box::new(SkillTool::new()),
    ];

    AgentBuilder::with_brain(Box::new(CoderBrain::new(llm.clone(), model)))
        .tools(toolbox)
        .build()
}
