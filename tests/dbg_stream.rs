//! 临时调试：打印 Responses API 流式 chunk 序列（验证后删除）。

use futures_util::StreamExt;
use mistake_agent::kernel::message::Message;
use mistake_agent::kernel::plugin::model::build_main_service;
use mistake_agent::kernel::plugin::services::{
    AbortSignal, ModelChunk, ModelKind, ModelRequest, ToolSchema,
};
use mistake_agent::kernel::settings::Settings;

#[tokio::test]
#[ignore]
async fn dump_chunks() {
    let settings = Settings::load().expect("settings");
    let svc = build_main_service(&settings);
    let tools = vec![ToolSchema {
        name: "demo__hello".into(),
        description: "打个招呼".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    }];
    let req = ModelRequest {
        model: ModelKind::Main,
        messages: vec![Message::user("必须调用 demo__hello 工具，然后简短说明结果")],
        tools: Some(tools.clone()),
        reasoning_effort: None,
        response_format: None,
        tool_choice: None,
    };
    let mut stream = svc.stream(&req, &AbortSignal::new()).await.expect("stream");
    let mut usage = None;
    let mut reasoning_id: Option<String> = None;
    let mut reasoning_text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk.expect("chunk ok") {
            ModelChunk::TextDelta(d) => eprintln!("TEXT {d:?}"),
            ModelChunk::ReasoningDelta(d) => {
                eprintln!("REASON {:?}", d.chars().take(20).collect::<String>());
                reasoning_text.push_str(&d);
            }
            ModelChunk::ReasoningItemStart { id } => {
                eprintln!("REASON_START {id}");
                reasoning_id = Some(id);
            }
            ModelChunk::ToolCallStart { index, name, .. } => eprintln!("TOOL_START {index} {name}"),
            ModelChunk::ToolCallDelta { index, data } => eprintln!("TOOL_DELTA {index} {data}"),
            ModelChunk::ItemDone { kind } => eprintln!("ITEM_DONE {kind:?}"),
            ModelChunk::Usage(u) => {
                eprintln!("USAGE {u:?}");
                usage = Some(u);
            }
            ModelChunk::Done => eprintln!("DONE"),
        }
    }
    eprintln!("FINAL usage: {usage:?}");
    eprintln!("REASONING_ID: {reasoning_id:?}");
    eprintln!("REASONING_TEXT_LEN: {}", reasoning_text.len());

    // 第二轮：把 reasoning（按 id）与工具调用回传给 API，验证 thinking 回传是否被接受。
    let rid = reasoning_id.expect("应有 reasoning id");
    let mut reasoning = Message::system("占位");
    reasoning.kind = mistake_agent::kernel::message::MessageKind::Reasoning {
        id: rid,
        text: reasoning_text.clone(),
    };
    let call = Message::tool_call(
        "demo::hello",
        serde_json::json!({}),
        Ok(serde_json::json!({"reply": "你好"})),
    );
    let req2 = ModelRequest {
        model: ModelKind::Main,
        messages: vec![Message::user("继续"), reasoning, call],
        tools: Some(tools),
        reasoning_effort: None,
        response_format: None,
        tool_choice: None,
    };
    let mut stream2 = svc
        .stream(&req2, &AbortSignal::new())
        .await
        .expect("第二轮 stream");
    while let Some(chunk) = stream2.next().await {
        match chunk {
            Ok(ModelChunk::TextDelta(d)) => eprintln!("R2 TEXT {d:?}"),
            Ok(ModelChunk::ItemDone { kind }) => eprintln!("R2 ITEM_DONE {kind:?}"),
            Ok(ModelChunk::Done) => eprintln!("R2 DONE"),
            Ok(other) => eprintln!("R2 OTHER {:?}", std::mem::discriminant(&other)),
            Err(e) => {
                eprintln!("R2 ERR {e:?}");
                panic!("第二轮回传失败：{e:?}");
            }
        }
    }
    eprintln!("第二轮回传成功");
}
