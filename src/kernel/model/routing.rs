use std::sync::Arc;

use crate::kernel::services::{
    AbortSignal, ModelError, ModelKind, ModelRequest, ModelResponse, ModelService, ModelStream,
};
use crate::kernel::settings::{Settings, Transport};

use super::*;

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

/// 配置热更新的模型服务（ADR-0015/0019）：持有共享 Settings，按 ModelKind 重建底层适配器。
/// `refresh()` 在 set_settings 保存成功后调用，下一次模型调用即用新配置；
/// 不重建时行为与构建期快照完全一致。
pub struct LiveSettingsModelService {
    settings: Arc<std::sync::RwLock<Settings>>,
    kind: ModelKind,
    current: std::sync::RwLock<Arc<dyn ModelService>>,
}

impl LiveSettingsModelService {
    pub fn new(settings: Arc<std::sync::RwLock<Settings>>, kind: ModelKind) -> Self {
        let snapshot = settings.read().expect("settings poisoned").clone();
        let current = match kind {
            ModelKind::Main => build_main_service(&snapshot),
            ModelKind::Vision => build_vision_service(&snapshot),
        };
        Self {
            settings,
            kind,
            current: std::sync::RwLock::new(current),
        }
    }

    /// 按当前 settings 重建底层适配器（set_settings 成功后调用）。
    pub fn refresh(&self) {
        let snapshot = self.settings.read().expect("settings poisoned").clone();
        let rebuilt = match self.kind {
            ModelKind::Main => build_main_service(&snapshot),
            ModelKind::Vision => build_vision_service(&snapshot),
        };
        *self.current.write().expect("model service poisoned") = rebuilt;
    }
}

#[async_trait::async_trait]
impl ModelService for LiveSettingsModelService {
    async fn stream(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelStream, ModelError> {
        let svc = self.current.read().expect("model service poisoned").clone();
        svc.stream(request, signal).await
    }

    async fn complete(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelResponse, ModelError> {
        let svc = self.current.read().expect("model service poisoned").clone();
        svc.complete(request, signal).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::message::Message;
    use crate::kernel::model::responses::{ResponsesModelService, SseParser};
    use crate::kernel::services::{ItemKind, ModelChunk};
    use futures_util::StreamExt;

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

    /// 回归测试（ticket 02）：DeepSeek Responses 真实事件形状——
    /// item 的类型在 `item` 子对象里，usage 在 `response.usage`。
    /// 用本地假 HTTP 服务喂探针抓到的 SSE 序列，验证消息/工具调用/usage 映射。
    #[tokio::test]
    async fn responses_stream_maps_real_event_shapes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let sse = concat!(
            "event: response.created\ndata: {\"type\":\"response.created\"}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc1\",\"call_id\":\"call_1\",\"name\":\"get_weather\",\"arguments\":\"\"},\"output_index\":1}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc1\",\"output_index\":1,\"delta\":\"{\\\"city\\\":\\\"北京\\\"}\"}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"fc1\",\"call_id\":\"call_1\",\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"北京\\\"}\"},\"output_index\":1}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"m1\"},\"output_index\":2}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"m1\",\"output_index\":2,\"delta\":\"北京今天晴天\"}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"m1\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"北京今天晴天\"}]},\"output_index\":2}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":84,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens\":27,\"output_tokens_details\":{\"reasoning_tokens\":17}}}}\n\n",
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let _ = sock.read(&mut buf).await;
            let header =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            sock.write_all(header.as_bytes()).await.unwrap();
            sock.write_all(sse.as_bytes()).await.unwrap();
            let _ = sock.shutdown().await;
        });

        let svc = ResponsesModelService::new(
            format!("http://{addr}"),
            "test-key".into(),
            "deepseek-v4-flash".into(),
        );
        let request = ModelRequest {
            model: ModelKind::Main,
            messages: vec![Message::user("北京天气？")],
            tools: None,
            reasoning_effort: None,
            response_format: None,
            tool_choice: None,
        };
        let mut stream = svc
            .stream(&request, &AbortSignal::new())
            .await
            .expect("stream 应成功");

        let mut text = String::new();
        let mut tool_name = String::new();
        let mut tool_args = String::new();
        let mut message_done = false;
        let mut call_done = false;
        let mut usage = None;
        while let Some(chunk) = stream.next().await {
            match chunk.expect("chunk 无错误") {
                ModelChunk::TextDelta(d) => text.push_str(&d),
                ModelChunk::ToolCallStart { name, .. } => tool_name = name,
                ModelChunk::ToolCallDelta { data, .. } => tool_args.push_str(&data),
                ModelChunk::ItemDone {
                    kind: ItemKind::Message,
                } => message_done = true,
                ModelChunk::ItemDone {
                    kind: ItemKind::FunctionCall,
                } => call_done = true,
                ModelChunk::ItemDone {
                    kind: ItemKind::Reasoning,
                } => {}
                ModelChunk::Usage(u) => usage = Some(u),
                ModelChunk::ReasoningDelta(_)
                | ModelChunk::ReasoningItemStart { .. }
                | ModelChunk::Done => {}
            }
        }
        assert_eq!(tool_name, "get_weather");
        assert_eq!(tool_args, r#"{"city":"北京"}"#);
        assert!(call_done);
        assert_eq!(text, "北京今天晴天");
        assert!(message_done);
        let usage = usage.expect("usage 应解析到");
        assert_eq!(usage.input_tokens, Some(84));
        assert_eq!(usage.output_tokens, Some(27));
        assert_eq!(usage.cached_tokens, Some(0));
        assert_eq!(usage.reasoning_tokens, Some(17));
    }
}
