//! model 服务适配器（ADR-0020）：
//! - 主模型：DeepSeek Responses API（POST /responses，SSE 语义事件，无状态）；
//! - 视觉模型：SiliconFlow Chat Completions（image_url base64 直读，仅提取内容不判分）。

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};

use crate::kernel::contract::full_to_wire;
use crate::kernel::message::{Message, MessageKind};
use crate::kernel::services::{
    AbortSignal, ItemKind, ModelChunk, ModelError, ModelKind, ModelRequest, ModelResponse,
    ModelService, ModelStream, ResponseFormat, TokenUsage, ToolSchema,
};
use crate::kernel::settings::{Settings, Transport};

// ---------- SSE 解析（Responses API 语义事件） ----------

struct SseEvent {
    name: String,
    data: String,
}

#[derive(Default)]
struct SseParser {
    buffer: Vec<u8>,
    event: String,
    data: String,
}

impl SseParser {
    fn push_chunk(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes[..line_bytes.len() - 1])
                .trim_end_matches('\r')
                .to_string();
            if line.is_empty() {
                if !self.event.is_empty() || !self.data.is_empty() {
                    events.push(SseEvent {
                        name: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data),
                    });
                }
            } else if let Some(v) = line.strip_prefix("event:") {
                self.event = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(v.trim());
            }
        }
        events
    }
}

// ---------- 通用工具 ----------

fn responses_endpoint(api_url: &str) -> String {
    let base = api_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/responses")
}

fn text_format(fmt: &ResponseFormat) -> Value {
    match fmt {
        ResponseFormat::JsonObject => json!({"type": "json_object"}),
        ResponseFormat::JsonSchema { name, schema } => json!({
            "type": "json_schema",
            "name": name,
            "schema": schema,
        }),
    }
}

fn parse_delta(data: &str) -> String {
    serde_json::from_str::<Value>(data)
        .ok()
        .and_then(|v| v["delta"].as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn tool_to_function(t: &ToolSchema) -> Value {
    json!({
        "type": "function",
        "name": t.name,
        "description": t.description,
        "parameters": t.input_schema,
    })
}

/// 内部 Message 树 → Responses API input items。
/// ToolCall 一条消息展开为 function_call + function_call_output（call_id = 消息 id）。
fn messages_to_responses_input(messages: &[Message]) -> Result<Vec<Value>, ModelError> {
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
            MessageKind::ToolCall {
                entry,
                params,
                result,
            } => {
                let call_id = msg.id.to_string();
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
fn messages_to_cc(messages: &[Message]) -> Vec<Value> {
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
            MessageKind::ToolCall {
                entry,
                params,
                result,
            } => {
                let call_id = msg.id.to_string();
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

fn map_status_error(status: reqwest::StatusCode, body: &str) -> ModelError {
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

fn reqwest_chain(e: &reqwest::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        out.push_str(&format!(" <- {s}"));
        src = s.source();
    }
    out
}

// ---------- DeepSeek Responses API 适配器 ----------

pub struct ResponsesModelService {
    client: Client,
    api_url: String,
    api_key: String,
    model: String,
}

impl ResponsesModelService {
    pub fn new(api_url: String, api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            // 无 IPv6 环境下强制走 IPv4（避免解析到 v6 后连接立即失败）。
            .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .build()
            .expect("reqwest client 构建失败");
        Self {
            client,
            api_url,
            api_key,
            model,
        }
    }

    fn build_body(&self, request: &ModelRequest) -> Result<Value, ModelError> {
        let mut body = json!({
            "model": self.model,
            "input": messages_to_responses_input(&request.messages)?,
            "stream": true,
        });
        if let Some(tools) = &request.tools {
            body["tools"] = json!(tools.iter().map(tool_to_function).collect::<Vec<_>>());
        }
        if let Some(effort) = &request.reasoning_effort {
            body["reasoning"] = json!({"effort": effort});
        }
        if let Some(fmt) = &request.response_format {
            body["text"] = json!({"format": text_format(fmt)});
        }
        Ok(body)
    }
}

#[async_trait::async_trait]
impl ModelService for ResponsesModelService {
    async fn stream(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelStream, ModelError> {
        let body = self.build_body(request)?;
        let url = responses_endpoint(&self.api_url);
        let response = match tokio::time::timeout(
            Duration::from_secs(60),
            self.client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(ModelError::Transport(reqwest_chain(&e))),
            Err(_) => return Err(ModelError::Timeout),
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(map_status_error(status, &text));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ModelChunk, ModelError>>(128);
        let mut byte_stream = response.bytes_stream();
        let cancel = signal.cancelled();
        tokio::spawn(async move {
            let mut parser = SseParser::default();
            let mut last_tool_index = 0usize;
            let mut done = false;
            loop {
                let next = tokio::select! {
                    chunk = byte_stream.next() => chunk,
                    _ = cancel.cancelled() => None,
                };
                let Some(chunk) = next else { break };
                match chunk {
                    Ok(bytes) => {
                        for ev in parser.push_chunk(&bytes) {
                            match ev.name.as_str() {
                                "response.output_item.added" => {
                                    if let Ok(v) = serde_json::from_str::<Value>(&ev.data)
                                        && v["type"] == "function_call"
                                    {
                                        last_tool_index += 1;
                                        let index = last_tool_index;
                                        let call_id =
                                            v["call_id"].as_str().unwrap_or_default().to_string();
                                        let name =
                                            v["name"].as_str().unwrap_or_default().to_string();
                                        let _ = tx
                                            .send(Ok(ModelChunk::ToolCallStart {
                                                index,
                                                call_id,
                                                name,
                                            }))
                                            .await;
                                    }
                                }
                                "response.output_item.done" => {
                                    if let Ok(v) = serde_json::from_str::<Value>(&ev.data) {
                                        match v["type"].as_str() {
                                            Some("message") => {
                                                let _ = tx
                                                    .send(Ok(ModelChunk::ItemDone {
                                                        kind: ItemKind::Message,
                                                    }))
                                                    .await;
                                            }
                                            Some("function_call") => {
                                                let _ = tx
                                                    .send(Ok(ModelChunk::ItemDone {
                                                        kind: ItemKind::FunctionCall,
                                                    }))
                                                    .await;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                "response.reasoning_text.delta" => {
                                    let _ = tx
                                        .send(Ok(ModelChunk::ReasoningDelta(parse_delta(&ev.data))))
                                        .await;
                                }
                                "response.output_text.delta" => {
                                    let _ = tx
                                        .send(Ok(ModelChunk::TextDelta(parse_delta(&ev.data))))
                                        .await;
                                }
                                "response.function_call_arguments.delta" => {
                                    if let Ok(v) = serde_json::from_str::<Value>(&ev.data) {
                                        let index = v["output_index"]
                                            .as_u64()
                                            .map(|i| i as usize)
                                            .unwrap_or(last_tool_index);
                                        let data =
                                            v["delta"].as_str().unwrap_or_default().to_string();
                                        let _ = tx
                                            .send(Ok(ModelChunk::ToolCallDelta { index, data }))
                                            .await;
                                    }
                                }
                                "response.completed" | "response.incomplete" => {
                                    if !done {
                                        if let Ok(v) = serde_json::from_str::<Value>(&ev.data) {
                                            let usage = parse_usage(&v["usage"]);
                                            let _ = tx.send(Ok(ModelChunk::Usage(usage))).await;
                                        }
                                        let _ = tx.send(Ok(ModelChunk::Done)).await;
                                        done = true;
                                    }
                                }
                                "response.failed" if !done => {
                                    let message = serde_json::from_str::<Value>(&ev.data)
                                        .ok()
                                        .and_then(|v| {
                                            v["error"]["message"].as_str().map(|s| s.to_string())
                                        })
                                        .unwrap_or_else(|| "响应失败".into());
                                    let _ = tx.send(Err(ModelError::Protocol(message))).await;
                                    done = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(ModelError::Transport(e.to_string()))).await;
                        break;
                    }
                }
            }
            if !done {
                let _ = tx.send(Ok(ModelChunk::Done)).await;
            }
        });

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

fn parse_usage(v: &Value) -> TokenUsage {
    TokenUsage {
        input_tokens: v["input_tokens"].as_u64(),
        output_tokens: v["output_tokens"].as_u64(),
        cached_tokens: v["input_tokens_details"]["cached_tokens"].as_u64(),
        reasoning_tokens: v["output_tokens_details"]["reasoning_tokens"].as_u64(),
    }
}

// ---------- SiliconFlow / Ollama Chat Completions 适配器（视觉模型） ----------

pub struct ChatCompletionsModelService {
    client: Client,
    api_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl ChatCompletionsModelService {
    pub fn new(api_url: String, api_key: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .build()
            .expect("reqwest client 构建失败");
        Self {
            client,
            api_url,
            api_key,
            model,
            max_tokens: 4096,
        }
    }

    async fn complete_inner(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelResponse, ModelError> {
        let url = format!("{}/chat/completions", self.api_url.trim_end_matches('/'));
        let mut body = json!({
            "model": self.model,
            "messages": messages_to_cc(&request.messages),
            "max_tokens": self.max_tokens,
        });
        if let Some(fmt) = &request.response_format {
            body["response_format"] = match fmt {
                ResponseFormat::JsonObject => json!({"type": "json_object"}),
                // Chat Completions 兜底：json_schema 降级为 json_object（判分走 Main/Responses，不依赖此路径）。
                ResponseFormat::JsonSchema { .. } => json!({"type": "json_object"}),
            };
        }
        let response = match tokio::time::timeout(
            Duration::from_secs(180),
            self.client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(ModelError::Transport(reqwest_chain(&e))),
            Err(_) => return Err(ModelError::Timeout),
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(map_status_error(status, &text));
        }
        if signal.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let data: Value = response
            .json()
            .await
            .map_err(|e| ModelError::Protocol(format!("响应解析失败：{e}")))?;
        let text = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let usage = TokenUsage {
            input_tokens: data["usage"]["prompt_tokens"].as_u64(),
            output_tokens: data["usage"]["completion_tokens"].as_u64(),
            ..Default::default()
        };
        Ok(ModelResponse {
            text,
            tool_calls: Vec::new(),
            usage: Some(usage),
        })
    }
}

#[async_trait::async_trait]
impl ModelService for ChatCompletionsModelService {
    async fn stream(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelStream, ModelError> {
        let response = self.complete_inner(request, signal).await?;
        let mut chunks: Vec<Result<ModelChunk, ModelError>> = Vec::new();
        if !response.text.is_empty() {
            chunks.push(Ok(ModelChunk::TextDelta(response.text)));
        }
        chunks.push(Ok(ModelChunk::Done));
        Ok(Box::new(futures_util::stream::iter(chunks)))
    }

    async fn complete(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelResponse, ModelError> {
        self.complete_inner(request, signal).await
    }
}

// ---------- 构建函数 ----------

/// 按 ModelKind 路由到主/视觉服务的组合适配器（ModelHandle 用）。
pub struct RoutingModelService {
    main: Arc<dyn ModelService>,
    vision: Arc<dyn ModelService>,
}

impl RoutingModelService {
    pub fn new(main: Arc<dyn ModelService>, vision: Arc<dyn ModelService>) -> Self {
        Self { main, vision }
    }
}

#[async_trait::async_trait]
impl ModelService for RoutingModelService {
    async fn stream(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelStream, ModelError> {
        match request.model {
            ModelKind::Main => self.main.stream(request, signal).await,
            ModelKind::Vision => self.vision.stream(request, signal).await,
        }
    }

    async fn complete(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelResponse, ModelError> {
        match request.model {
            ModelKind::Main => self.main.complete(request, signal).await,
            ModelKind::Vision => self.vision.complete(request, signal).await,
        }
    }
}

pub fn build_main_service(settings: &Settings) -> Arc<dyn ModelService> {
    let cfg = &settings.main_model;
    let model = cfg
        .model
        .clone()
        .unwrap_or_else(|| "deepseek-v4-flash".into());
    match cfg.transport.unwrap_or_default() {
        Transport::Responses => Arc::new(ResponsesModelService::new(
            cfg.api_url.clone(),
            cfg.api_key.clone(),
            model,
        )),
        Transport::ChatCompletions => Arc::new(ChatCompletionsModelService::new(
            cfg.api_url.clone(),
            cfg.api_key.clone(),
            model,
        )),
    }
}

pub fn build_vision_service(settings: &Settings) -> Arc<dyn ModelService> {
    let cfg = &settings.vision_model;
    let model = cfg
        .model
        .clone()
        .unwrap_or_else(|| "Qwen/Qwen3-VL-32B-Instruct".into());
    Arc::new(ChatCompletionsModelService::new(
        cfg.api_url.clone(),
        cfg.api_key.clone(),
        model,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::message::Message;

    #[test]
    fn sse_parser_handles_events() {
        let mut parser = SseParser::default();
        let events = parser.push_chunk(
            b"event: response.output_text.delta\ndata: {\"delta\":\"a\"}\n\nevent: response.output_text.delta\ndata: {\"delta\":\"b\"}\n\n",
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].name, "response.output_text.delta");
        assert_eq!(events[0].data, r#"{"delta":"a"}"#);
    }

    #[test]
    fn responses_input_expands_tool_call() {
        let mut call =
            Message::tool_call("demo::hello", json!({"x": 1}), Ok(json!({"reply": "hi"})));
        let _ = &mut call;
        let user = Message::user("你好");
        let items = messages_to_responses_input(&[user, call]).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["name"], "demo_hello");
        assert_eq!(items[2]["type"], "function_call_output");
    }
}
