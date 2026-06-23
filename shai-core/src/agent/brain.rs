use std::sync::Arc;
use async_trait::async_trait;
use openai_dive::v1::resources::chat::ChatMessage;
use shai_llm::ToolCallMethod;
use tokio::sync::RwLock;

use crate::tools::types::AnyToolBox;
use super::error::AgentError;


/// ThinkerContext is the agent internal state
pub struct ThinkerContext {
    pub trace:           Arc<RwLock<Vec<ChatMessage>>>,
    pub available_tools: AnyToolBox,
    pub method:          ToolCallMethod,
    pub max_trace_chars: usize,
    pub temperature:      f32,
}

/// ThinkerFlowControl drives the agentic flow
#[derive(Debug, Clone)]
pub enum ThinkerFlowControl {
    AgentContinue,
    AgentPause
}

/// This structure pilot the flow of the Agent
/// If tool_call are present in the chat message, the flow attribute is ignored
/// If no tool_call is present in the chat message, flow will pilot wether the agent pause or continue
#[derive(Debug, Clone)]
pub struct ThinkerDecision {
    pub message: ChatMessage,
    pub flow:    ThinkerFlowControl,
    pub token_usage: Option<(u32, u32, u32)>, // (input_tokens, output_tokens, cached_tokens)
}

impl ThinkerDecision {
    pub fn new(message: ChatMessage) -> Self {
        ThinkerDecision{
            message,
            flow: ThinkerFlowControl::AgentPause,
            token_usage: None,
        }
    }

    pub fn agent_continue(message: ChatMessage) -> Self {
        ThinkerDecision{
            message,
            flow: ThinkerFlowControl::AgentContinue,
            token_usage: None,
        }
    }

    pub fn agent_pause(message: ChatMessage) -> Self {
        ThinkerDecision{
            message,
            flow: ThinkerFlowControl::AgentPause,
            token_usage: None,
        }
    }

    pub fn agent_continue_with_tokens(message: ChatMessage, input_tokens: u32, output_tokens: u32, cached_tokens: u32) -> Self {
        ThinkerDecision{
            message,
            flow: ThinkerFlowControl::AgentContinue,
            token_usage: Some((input_tokens, output_tokens, cached_tokens)),
        }
    }

    pub fn agent_pause_with_tokens(message: ChatMessage, input_tokens: u32, output_tokens: u32, cached_tokens: u32) -> Self {
        ThinkerDecision{
            message,
            flow: ThinkerFlowControl::AgentPause,
            token_usage: Some((input_tokens, output_tokens, cached_tokens)),
        }
    }

    pub fn unwrap(self) -> ChatMessage {
        self.message
    }
}

/// Core thinking interface - pure decision making
#[async_trait]
pub trait Brain: Send + Sync {
    /// This method is called at every step of the agent to decide next step
    /// note that if the message contains toolcall, it will always continue
    async fn next_step(&mut self, context: ThinkerContext) -> Result<ThinkerDecision, AgentError>;
}


