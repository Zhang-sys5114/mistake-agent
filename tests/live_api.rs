//! 真实 API 验收（用户要求：任何验收点都直接接 API）。
//! 运行：cargo test --test live_api -- --ignored --nocapture
//! 需要 ~/Documents/.mistake-agent/settings.json 中配置真实 key；缺失时自动跳过。

use std::sync::Arc;
use std::time::Duration;

use mistake_agent::kernel::dispatch::Caller;
use mistake_agent::kernel::events::MemoryEventSink;
use mistake_agent::kernel::rpc::{Kernel, Method, RpcRequest};
use mistake_agent::kernel::settings::Settings;
use serde_json::json;

fn real_api_ready() -> bool {
    match Settings::load() {
        Ok(s) => !s.main_model.api_key.is_empty() && !s.vision_model.api_key.is_empty(),
        Err(_) => false,
    }
}

async fn wait_idle(kernel: &Arc<Kernel>, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let req = RpcRequest {
            id: 999,
            method: Method::GetState,
        };
        if let Ok(Some(frame)) = kernel.handle(req).await {
            let s = serde_json::to_string(&frame).unwrap_or_default();
            if s.contains("\"idle\"") {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 链路 1：hello 回合 —— send_user_message → 主模型 Responses API 流式 → 回合结束。
#[tokio::test]
#[ignore]
async fn hello_turn_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let req = RpcRequest {
        id: 1,
        method: Method::SendUserMessage {
            text: "你好，请打个招呼".into(),
        },
    };
    let frame = kernel.handle(req).await.expect("请求失败");
    assert!(
        serde_json::to_string(&frame)
            .unwrap()
            .contains("\"accepted\":true")
    );
    assert!(
        wait_idle(&kernel, Duration::from_secs(120)).await,
        "回合 120s 内未结束"
    );
    let events = events.take();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, mistake_agent::kernel::events::Event::MessageDelta { .. })),
        "应有流式消息增量事件"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, mistake_agent::kernel::events::Event::TurnEnd { .. })),
        "应有回合结束事件"
    );
    eprintln!("hello 回合真实链路通过，事件数：{}", events.len());
}

/// 链路 2：场景一 —— 三套作业样例端到端（图片 → 视觉 OCR → 主模型判分 → 错题归档）。
#[tokio::test]
#[ignore]
async fn grading_upload_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let samples_dir = std::path::Path::new("samples");
    if !samples_dir.exists() {
        eprintln!("SKIP: samples/ 目录不存在");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let dispatch = kernel.dispatch();

    let mut files: Vec<_> = std::fs::read_dir(samples_dir)
        .expect("读取 samples 失败")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "png" | "jpg" | "jpeg"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "samples/ 下没有图片");

    for file in &files {
        eprintln!("=== 批改样例：{file:?} ===");
        let result = dispatch
            .call_tool(
                "grading::upload",
                json!({ "file": file.to_string_lossy() }),
                Caller::User,
            )
            .await
            .unwrap_or_else(|e| panic!("grading::upload 失败 {file:?}：{e:?}"));
        assert!(
            result["total"].as_u64().unwrap_or(0) >= 1,
            "{file:?} 至少识别 1 题"
        );
        eprintln!(
            "{}：共 {} 题，对 {}，错 {}，归档 {}",
            file.file_name().unwrap().to_string_lossy(),
            result["total"],
            result["correct_count"],
            result["wrong_count"],
            result["archived_mistakes"],
        );
    }

    let list = dispatch
        .call_tool("grading::list", json!({}), Caller::User)
        .await
        .expect("grading::list 失败");
    eprintln!("错题本累计：{} 条", list["count"].as_u64().unwrap_or(0));
    eprintln!("场景一（三套样例）真实链路通过");
}
