//! Agent loop（Q13 / ADR-0005）：LLM 唯一决策者，kernel 执行工具调用。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::audit::{AuditRecord, Auditor};
use crate::kernel::contract::{ToolError, ToolErrorCode};
use crate::kernel::dispatch::{Caller, Dispatch};
use crate::kernel::events::{Event, EventSink};
use crate::kernel::message::{Message, MessageId, MessageKind, append_to_path};
use crate::kernel::prompt::agent_system_prompt;
use crate::kernel::services::{
    AbortSignal, ItemKind, ModelChunk, ModelKind, ModelRequest, ModelService, TokenUsage,
    ToolSchema,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    ModelUnavailable,
    SettingsChanged,
    AuditFailure,
    PluginRequested(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Natural,
    ToolCallLimit,
    ConsecutiveFailures,
    TurnTimeout,
    UserAborted,
    InternalAbort { reason: InterruptReason },
}

pub struct TurnInput {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub signal: AbortSignal,
    pub turn_budget: Duration,
}

#[derive(Debug)]
pub struct TurnOutcome {
    pub messages: Vec<Message>,
    pub stop_reason: StopReason,
    pub tool_calls: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("模型错误：{0}")]
    Model(String),
    #[error("内部错误：{0}")]
    Internal(String),
}

struct ToolCallAcc {
    name: String,
    arguments: String,
}

pub struct AgentLoop {
    model: Arc<dyn ModelService>,
    dispatch: Arc<Dispatch>,
    auditor: Auditor,
    events: Arc<dyn EventSink>,
    max_tool_calls: usize,
    max_consecutive_failures: usize,
}

impl AgentLoop {
    pub fn new(
        model: Arc<dyn ModelService>,
        dispatch: Arc<Dispatch>,
        auditor: Auditor,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            model,
            dispatch,
            auditor,
            events,
            max_tool_calls: 25,
            max_consecutive_failures: 3,
        }
    }

    pub async fn run_turn(&self, input: TurnInput) -> Result<TurnOutcome, LoopError> {
        let start_len = input.messages.len();
        let mut conversation = input.messages;
        let turn_deadline = Instant::now() + input.turn_budget;
        let mut tool_calls = 0usize;
        let mut consecutive_failures = 0usize;
        let mut last_code: Option<ToolErrorCode> = None;

        let stop_reason = loop {
            if input.signal.is_cancelled() {
                break StopReason::UserAborted;
            }
            if Instant::now() >= turn_deadline {
                break StopReason::TurnTimeout;
            }

            // 系统提示每次请求注入（不落消息树），保证无状态 API 拿到完整人格设定。
            let mut req_messages = vec![Message::system(agent_system_prompt())];
            req_messages.extend(conversation.iter().cloned());
            let request = ModelRequest {
                model: ModelKind::Main,
                messages: req_messages,
                tools: Some(input.tools.clone()),
                reasoning_effort: None,
                response_format: None,
            };
            let started = Instant::now();
            let mut stream = match self.model.stream(&request, &input.signal).await {
                Ok(s) => s,
                Err(e) => {
                    self.auditor.record(AuditRecord::LlmCall {
                        provider: "main".into(),
                        model: "main".into(),
                        kind: "stream".into(),
                        tokens_in: None,
                        tokens_out: None,
                        duration_ms: started.elapsed().as_millis() as u64,
                        ok: false,
                    });
                    if e.is_systemic() {
                        break StopReason::InternalAbort {
                            reason: InterruptReason::ModelUnavailable,
                        };
                    }
                    if matches!(e, crate::kernel::services::ModelError::Cancelled) {
                        break StopReason::UserAborted;
                    }
                    return Err(LoopError::Model(e.to_string()));
                }
            };

            let mut pending_bubble: Option<Message> = None;
            let mut calls: BTreeMap<usize, ToolCallAcc> = BTreeMap::new();
            let mut calls_done: Vec<(usize, ToolCallAcc)> = Vec::new();
            let mut usage: Option<TokenUsage> = None;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(ModelChunk::TextDelta(delta)) => {
                        let entry = pending_bubble.get_or_insert_with(|| Message {
                            id: MessageId::new(),
                            parent_id: None,
                            kind: MessageKind::Assistant {
                                text: String::new(),
                            },
                            created_at: chrono::Utc::now(),
                        });
                        if let MessageKind::Assistant { text } = &mut entry.kind {
                            text.push_str(&delta);
                        }
                        self.events.emit(Event::MessageDelta {
                            message_id: entry.id,
                            delta,
                        });
                    }
                    Ok(ModelChunk::ReasoningDelta(delta)) => {
                        self.events.emit(Event::ReasoningDelta { delta });
                    }
                    Ok(ModelChunk::ToolCallStart {
                        index,
                        call_id: _call_id,
                        name,
                    }) => {
                        calls.insert(
                            index,
                            ToolCallAcc {
                                name,
                                arguments: String::new(),
                            },
                        );
                    }
                    Ok(ModelChunk::ToolCallDelta { index, data }) => {
                        if let Some(acc) = calls.get_mut(&index) {
                            acc.arguments.push_str(&data);
                        }
                    }
                    Ok(ModelChunk::ItemDone {
                        kind: ItemKind::Message,
                    }) => {
                        if let Some(bubble) = pending_bubble.take() {
                            let text = match &bubble.kind {
                                MessageKind::Assistant { text } => text.clone(),
                                _ => String::new(),
                            };
                            if !text.is_empty() {
                                self.auditor.record(AuditRecord::MessageCompleted {
                                    message_id: bubble.id,
                                });
                                append_to_path(&mut conversation, bubble);
                            }
                        }
                    }
                    Ok(ModelChunk::ItemDone {
                        kind: ItemKind::FunctionCall,
                    }) => {
                        // 收集完成顺序（BTreeMap 已按 index 排序，执行按输出顺序串行）。
                        if let Some((idx, acc)) = calls.pop_first() {
                            calls_done.push((idx, acc));
                        }
                    }
                    Ok(ModelChunk::Usage(u)) => {
                        usage = Some(u);
                    }
                    Ok(ModelChunk::Done) => break,
                    Err(e) => {
                        self.auditor.record(AuditRecord::LlmCall {
                            provider: "main".into(),
                            model: "main".into(),
                            kind: "stream".into(),
                            tokens_in: usage.as_ref().and_then(|u| u.input_tokens),
                            tokens_out: usage.as_ref().and_then(|u| u.output_tokens),
                            duration_ms: started.elapsed().as_millis() as u64,
                            ok: false,
                        });
                        if e.is_systemic() {
                            return Ok(TurnOutcome {
                                messages: conversation[start_len..].to_vec(),
                                stop_reason: StopReason::InternalAbort {
                                    reason: InterruptReason::ModelUnavailable,
                                },
                                tool_calls,
                            });
                        }
                        return Err(LoopError::Model(e.to_string()));
                    }
                }
            }

            // 流被截断时补收尾：未关闭的气泡不落盘，半截调用丢弃。
            self.auditor.record(AuditRecord::LlmCall {
                provider: "main".into(),
                model: "main".into(),
                kind: "stream".into(),
                tokens_in: usage.as_ref().and_then(|u| u.input_tokens),
                tokens_out: usage.as_ref().and_then(|u| u.output_tokens),
                duration_ms: started.elapsed().as_millis() as u64,
                ok: true,
            });

            if input.signal.is_cancelled() {
                break StopReason::UserAborted;
            }

            if calls_done.is_empty() {
                break StopReason::Natural;
            }

            let mut stop: Option<StopReason> = None;
            for (_idx, acc) in calls_done {
                tool_calls += 1;
                if tool_calls > self.max_tool_calls {
                    stop = Some(StopReason::ToolCallLimit);
                    break;
                }
                let wire_name = acc.name.clone();
                let full_name = self.dispatch.resolve_wire(&wire_name).unwrap_or_default();
                let params: Value = serde_json::from_str(&acc.arguments).unwrap_or(Value::Null);
                self.events.emit(Event::ToolStart {
                    entry: full_name.clone(),
                    icon: self.dispatch.entry_icon(&full_name),
                });
                let result = if full_name.is_empty() {
                    Err(ToolError::unknown_tool(&wire_name))
                } else {
                    self.dispatch
                        .call_tool(&full_name, params.clone(), Caller::Model)
                        .await
                };
                self.events.emit(Event::ToolEnd {
                    entry: full_name.clone(),
                    ok: result.is_ok(),
                });

                match &result {
                    Ok(_) => consecutive_failures = 0,
                    Err(e) => {
                        if Some(e.code) == last_code {
                            consecutive_failures += 1;
                        } else {
                            consecutive_failures = 1;
                        }
                        last_code = Some(e.code);
                        if consecutive_failures >= self.max_consecutive_failures {
                            append_to_path(
                                &mut conversation,
                                Message::tool_call(full_name, params, result),
                            );
                            stop = Some(StopReason::ConsecutiveFailures);
                            break;
                        }
                    }
                }
                append_to_path(
                    &mut conversation,
                    Message::tool_call(full_name, params, result),
                );
            }
            if let Some(s) = stop {
                break s;
            }
        };

        let outcome = TurnOutcome {
            messages: conversation[start_len..].to_vec(),
            stop_reason,
            tool_calls,
        };
        self.events.emit(Event::TurnEnd {
            stop_reason: outcome.stop_reason.clone(),
        });
        self.auditor.record(AuditRecord::TurnEnded {
            stop_reason: format!("{:?}", outcome.stop_reason),
            tool_calls,
        });
        Ok(outcome)
    }
}
