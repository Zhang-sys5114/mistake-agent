//! model 服务适配器（ADR-0020）：
//! - 主模型：DeepSeek Responses API（POST /responses，SSE 语义事件，无状态）；
//! - 视觉模型：SiliconFlow Chat Completions（image_url base64 直读，仅提取内容不判分）。

use std::error::Error;

use serde_json::{Value, json};

use crate::kernel::contract::full_to_wire;
use crate::kernel::message::{Message, MessageKind};
use crate::kernel::plugin::services::{ModelError, ResponseFormat, ToolSchema};

// ---------- SSE 解析（Responses API 语义事件） ----------

mod chat;
mod responses;
mod routing;

pub use chat::ChatCompletionsModelService;
pub use responses::ResponsesModelService;
pub use routing::{
    LiveSettingsModelService, RoutingModelService, build_main_service, build_vision_service,
};

// ---------- 内核插件入口（ADR-0035）：model 服务身份声明 ----------
//
// 服务实例（双模型 LiveSettingsModelService + RoutingModelService）由 Kernel::new
// 引导构造（依赖 settings 热更新），注册表侧只声明 ServiceId 提供与 namespace 占用。

use crate::kernel::context::KernelContext;
use crate::kernel::contract::{Info, PluginError};
use crate::kernel::plugin::services::ServiceId;
use crate::kernel::registry::{KernelDescriptor, KernelPlugin};

pub struct ModelPlugin;

impl KernelPlugin for ModelPlugin {
    fn info() -> Info {
        Info {
            namespace: "model".into(),
            provides: vec![ServiceId::Model],
            ..Default::default()
        }
    }

    fn register(_ctx: KernelContext<'_>) -> Result<(), PluginError> {
        Ok(())
    }
}

pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<ModelPlugin>()
}

pub(crate) fn responses_endpoint(api_url: &str) -> String {
    let base = api_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/responses")
}

pub(crate) fn text_format(fmt: &ResponseFormat) -> Value {
    match fmt {
        ResponseFormat::JsonObject => json!({"type": "json_object"}),
        ResponseFormat::JsonSchema { name, schema } => json!({
            "type": "json_schema",
            "name": name,
            "schema": schema,
        }),
    }
}

pub(crate) fn parse_delta(data: &str) -> String {
    serde_json::from_str::<Value>(data)
        .ok()
        .and_then(|v| v["delta"].as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

pub(crate) fn tool_to_function(t: &ToolSchema) -> Value {
    json!({
        "type": "function",
        "name": t.name,
        "description": t.description,
        "parameters": t.input_schema,
    })
}

/// 内部 Message 树 → Responses API input items。
/// ToolCall 一条消息展开为 function_call + function_call_output（call_id = 消息 id）。
pub(crate) fn messages_to_responses_input(messages: &[Message]) -> Result<Vec<Value>, ModelError> {
    let mut items = Vec::new();
    for msg in messages {
        match &msg.kind {
            MessageKind::User { text, .. } => items.push(json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            })),
            MessageKind::Assistant { text } => items.push(json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}],
            })),
            MessageKind::System { text } => items.push(json!({
                "type": "message",
                "role": "system",
                "content": [{"type": "input_text", "text": text}],
            })),
            // thinking 模式要求把推理 item 按 id 回传，否则下一轮协议报错。
            MessageKind::Reasoning { id, .. } => items.push(json!({
                "type": "reasoning",
                "id": id,
            })),
            MessageKind::ToolCall {
                entry,
                params,
                result,
                call_id,
            } => {
                let call_id = if call_id.is_empty() {
                    msg.id.to_string()
                } else {
                    call_id.clone()
                };
                let arguments = serde_json::to_string(params)
                    .map_err(|e| ModelError::Protocol(format!("参数序列化失败：{e}")))?;
                items.push(json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": full_to_wire(entry),
                    "arguments": arguments,
                }));
                let output = match result {
                    Ok(v) => serde_json::to_string(v)
                        .map_err(|e| ModelError::Protocol(format!("结果序列化失败：{e}")))?,
                    Err(e) => serde_json::to_string(&json!({"error": e}))
                        .map_err(|e| ModelError::Protocol(format!("错误序列化失败：{e}")))?,
                };
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
        }
    }
    Ok(items)
}

/// 内部 Message 树 → Chat Completions messages（视觉模型支持 image_url base64）。
pub(crate) fn messages_to_cc(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in messages {
        match &msg.kind {
            MessageKind::User { text, attachments } => {
                let mut content: Vec<Value> = Vec::new();
                for att in attachments {
                    content.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:{};base64,{}", att.mime, att.data_base64),
                            "detail": "high",
                        },
                    }));
                }
                if !text.is_empty() || content.is_empty() {
                    content.push(json!({"type": "text", "text": text}));
                }
                out.push(json!({"role": "user", "content": content}));
            }
            MessageKind::Assistant { text } => {
                out.push(json!({"role": "assistant", "content": text}));
            }
            MessageKind::System { text } => {
                out.push(json!({"role": "system", "content": text}));
            }
            // Chat Completions 无 reasoning 概念：忽略（Ollama 等兼容端）。
            MessageKind::Reasoning { .. } => {}
            MessageKind::ToolCall {
                entry,
                params,
                result,
                call_id,
            } => {
                let call_id = if call_id.is_empty() {
                    msg.id.to_string()
                } else {
                    call_id.clone()
                };
                let arguments = serde_json::to_string(params).unwrap_or_else(|_| "{}".into());
                out.push(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": full_to_wire(entry),
                            "arguments": arguments,
                        },
                    }],
                }));
                let output = match result {
                    Ok(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".into()),
                    Err(e) => {
                        serde_json::to_string(&json!({"error": e})).unwrap_or_else(|_| "{}".into())
                    }
                };
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
        }
    }
    out
}

pub(crate) fn map_status_error(status: reqwest::StatusCode, body: &str) -> ModelError {
    let body = body.chars().take(500).collect::<String>();
    match status.as_u16() {
        401 => ModelError::AuthFailed(body),
        402 => ModelError::QuotaExceeded(body),
        404 => ModelError::ModelNotFound(body),
        429 => ModelError::RateLimited(body),
        400 | 422 => ModelError::Protocol(body),
        _ => ModelError::Transport(format!("HTTP {status}: {body}")),
    }
}

pub(crate) fn reqwest_chain(e: &reqwest::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        out.push_str(&format!(" <- {s}"));
        src = s.source();
    }
    out
}

// ---------- DeepSeek Responses API 适配器 ----------
