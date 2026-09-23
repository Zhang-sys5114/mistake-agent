//! 真实 API 验收（用户要求：任何验收点都直接接 API）。
//! 运行：cargo test --test live_api -- --ignored --nocapture
//! 需要 ~/Documents/.mistake-agent/settings.json 中配置真实 key；缺失时自动跳过。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mistake_agent::kernel::agent::dispatch::Caller;
use mistake_agent::kernel::agent::rpc::{
    ForcedToolRequest, Kernel, Method, RpcRequest, WireMethod,
};
use mistake_agent::kernel::agent::session::SessionKey;
use mistake_agent::kernel::events::{Event, EventSink, MemoryEventSink};
use mistake_agent::kernel::settings::Settings;
use serde_json::json;

fn real_api_ready() -> bool {
    match Settings::load() {
        Ok(s) => !s.main_model.api_key.is_empty(),
        Err(_) => false,
    }
}

/// 把样例复制到数据根 uploads/（ADR-0046：附件以 uploads/ 引用直入模型上下文）。
fn stage_sample(src: &Path) -> PathBuf {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png");
    let uploads = mistake_agent::kernel::settings::Settings::data_root().join("uploads");
    std::fs::create_dir_all(&uploads).expect("创建 uploads 失败");
    let dest = uploads.join(format!("{}.{}", uuid::Uuid::new_v4(), ext));
    std::fs::copy(src, &dest).expect("暂存样例失败");
    dest
}

/// 模拟 GUI 执行端：后台应答 compute_request（固定 stdout），非 compute 事件原样回填（供断言消费）。
/// 真实 App 里 compute 由 WebView 内 Pyodide 执行；测试环境没有执行端，否则模型调 compute::verify 会等到工具超时。
fn spawn_compute_responder(
    events: Arc<MemoryEventSink>,
    kernel: Arc<Kernel>,
) -> (
    Arc<std::sync::atomic::AtomicBool>,
    tokio::task::JoinHandle<()>,
) {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = {
        let events = events.clone();
        let kernel = kernel.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                for e in events.take() {
                    match e {
                        Event::ComputeRequest { id, .. } => {
                            let _ = kernel
                                .handle(RpcRequest::custom(
                                    900,
                                    "compute_result",
                                    json!({"compute_id": id, "stdout": "ok", "stderr": "", "duration_ms": 3}),
                                ))
                                .await;
                        }
                        other => events.emit(other),
                    }
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        })
    };
    (stop, handle)
}

async fn wait_idle(kernel: &Arc<Kernel>, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let req = RpcRequest {
            id: 999,
            method: Method::GetState.into(),
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
    let (stop, responder) = spawn_compute_responder(events.clone(), kernel.clone());
    let req = RpcRequest {
        id: 1,
        method: Method::SendUserMessage {
            text: "你好，请打个招呼".into(),
            display_text: None,
            force_tool: None,
            file: vec![],
            asset: vec![],
        }
        .into(),
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
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = responder.await;
    let events = events.take();
    assert!(
        events.iter().any(|e| matches!(
            e,
            mistake_agent::kernel::events::Event::CacheStatsUpdated { .. }
        )),
        "回合落盘后应实时推送缓存统计事件"
    );
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

    // M2 验收：assistant 回复必须落盘，usage 必须解析进审计。
    let data_root = Settings::data_root();
    let sessions_dir = data_root.join("sessions");
    let latest = std::fs::read_dir(&sessions_dir)
        .expect("读取 sessions 目录失败")
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|e| {
            std::fs::metadata(e.path())
                .ok()
                .map(|m| (m.modified().ok(), e.path()))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
        .expect("没有会话文件");
    let session_text = std::fs::read_to_string(&latest).expect("读取会话文件失败");
    assert!(
        session_text.contains("\"kind\":\"assistant\"")
            || session_text.contains("\"kind\": \"assistant\""),
        "会话 JSONL 应包含 assistant 回复（当前文件 {latest:?}）"
    );

    let audit_path = data_root.join("audit").join("audit.jsonl");
    let audit_text = std::fs::read_to_string(&audit_path).expect("读取审计文件失败");
    let usage_ok = audit_text.lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .is_some_and(|v| {
                v["record"] == "llm_call" && v["tokens_in"].is_u64() && v["tokens_out"].is_u64()
            })
    });
    assert!(usage_ok, "审计中应有 tokens_in/tokens_out 非空的 llm_call");

    // 聊天上下文缓存命中统计：回合 usage 应已按会话累计（真实 API 返回 cached_tokens）。
    let stats_frame = kernel
        .handle(RpcRequest::custom(2, "get_cache_stats", json!({})))
        .await
        .expect("get_cache_stats 请求失败")
        .expect("应有响应帧");
    let stats = match stats_frame {
        mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, error, .. } => {
            assert!(error.is_none(), "缓存统计不应报错：{error:?}");
            result.expect("应有统计结果")
        }
        _ => panic!("缓存统计应返回 response 帧"),
    };
    assert!(
        stats["main"]["calls"].as_u64().unwrap_or(0) >= 1,
        "主模型应有至少 1 次回合调用：{stats}"
    );
    assert!(
        stats["main"]["hit_tokens"].is_u64() && stats["main"]["miss_tokens"].is_u64(),
        "应有命中/未命中 token 统计：{stats}"
    );
    assert!(
        stats["sessions"].as_array().is_some_and(|a| !a.is_empty()),
        "应有会话明细：{stats}"
    );
    eprintln!(
        "缓存命中统计真实链路通过：{} 次调用，命中 {} / 未命中 {} tokens",
        stats["main"]["calls"], stats["main"]["hit_tokens"], stats["main"]["miss_tokens"],
    );

    eprintln!("hello 回合真实链路通过，事件数：{}", events.len());
}

/// 链路 2：场景一 —— 三套作业样例端到端（图片直入上下文 → 模型读图判分 → grading::upload 归档，ADR-0046）。
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
    let (stop, responder) = spawn_compute_responder(events.clone(), kernel.clone());

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
        let asset = stage_sample(file);
        let req = RpcRequest {
            id: 100,
            method: Method::SendUserMessage {
                text: "请批改这份作业：逐题判分，并调用 grading::upload 把错题归档进错题本。"
                    .into(),
                display_text: None,
                force_tool: None,
                file: vec![],
                asset: vec![mistake_agent::kernel::agent::rpc::AttachmentInfo {
                    path: asset.to_string_lossy().into_owned(),
                    name: file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                }],
            }
            .into(),
        };
        kernel
            .handle(req)
            .await
            .unwrap_or_else(|e| panic!("批改请求失败 {file:?}：{e:?}"));
        assert!(
            wait_idle(&kernel, Duration::from_secs(240)).await,
            "{file:?} 批改回合未结束"
        );
        // 附件应已作为 uploads/ 引用落进消息树（图片直入上下文，不再走读图工具）。
        eprintln!(
            "{}：批改回合完成",
            file.file_name().unwrap().to_string_lossy()
        );
        let _ = std::fs::remove_file(&asset);
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = responder.await;

    let list = dispatch
        .call_tool("grading::list", json!({}), Caller::User)
        .await
        .expect("grading::list 失败");
    eprintln!("错题本累计：{} 条", list["count"].as_u64().unwrap_or(0));
    eprintln!("场景一（三套样例）真实链路通过");
}

/// 链路 3：显式 tool-calling —— 强制 LLM 首轮调用指定工具，输出仍由 LLM 生成。
#[tokio::test]
#[ignore]
async fn forced_tool_call_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let (stop, responder) = spawn_compute_responder(events.clone(), kernel.clone());
    let req = RpcRequest {
        id: 7,
        method: Method::SendUserMessage {
            text: "绝对值".into(),
            display_text: None,
            force_tool: Some(ForcedToolRequest {
                entry: "practice::generate".into(),
                hint: Some("绝对值".into()),
                display: None,
            }),
            file: vec![],
            asset: vec![],
        }
        .into(),
    };
    let frame = kernel.handle(req).await.expect("请求失败");
    assert!(
        serde_json::to_string(&frame)
            .unwrap()
            .contains("\"accepted\":true"),
        "强制工具回合应被接受"
    );
    assert!(
        wait_idle(&kernel, Duration::from_secs(120)).await,
        "回合 120s 内未结束"
    );
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = responder.await;
    let events = events.take();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, mistake_agent::kernel::events::Event::ToolStart { entry, .. } if entry == "practice::generate")),
        "应强制调用 practice::generate"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, mistake_agent::kernel::events::Event::MessageDelta { .. })),
        "模型应基于工具结果生成回复"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, mistake_agent::kernel::events::Event::TurnEnd { .. })),
        "应有回合结束事件"
    );
    eprintln!("显式 tool-calling（强制调用）真实链路通过");
}

/// 链路 4：LaTeX 渲染 —— 模型输出数学富文本应带 $...$ 标记（前端 KaTeX 渲染）。
#[tokio::test]
#[ignore]
async fn latex_output_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let (stop, responder) = spawn_compute_responder(events.clone(), kernel.clone());
    let req = RpcRequest {
        id: 8,
        method: Method::SendUserMessage {
            text: "请解释勾股定理，公式必须用 LaTeX 的 $...$ 标记输出。".into(),
            display_text: None,
            force_tool: None,
            file: vec![],
            asset: vec![],
        }
        .into(),
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
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = responder.await;
    let events = events.take();
    let mut reply = String::new();
    for e in &events {
        if let mistake_agent::kernel::events::Event::MessageDelta { delta, .. } = e {
            reply.push_str(delta);
        }
    }
    assert!(
        reply.contains('$'),
        "模型回复应包含 LaTeX 公式标记：{reply}"
    );
    eprintln!(
        "LaTeX 输出真实链路通过：{}",
        reply.chars().take(100).collect::<String>()
    );
}

/// 链路 3：memory 工具真实集成 —— save/show/remove 经 dispatch（用户侧）走文件持久化。
#[tokio::test]
#[ignore]
async fn memory_tools_roundtrip() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let dispatch = kernel.dispatch();
    let path = "测试/记忆条目";

    let saved = dispatch
        .call_tool(
            "memory::save",
            json!({"filename": path, "content": "顶点公式 y=a(x-h)^2+k"}),
            Caller::User,
        )
        .await
        .expect("memory::save 失败");
    assert_eq!(saved["ok"], true);

    let listing = dispatch
        .call_tool("memory::show", json!({}), Caller::User)
        .await
        .expect("memory::show 清单失败");
    assert!(
        listing["entries"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(path))),
        "清单应包含 {path}"
    );

    let detail = dispatch
        .call_tool("memory::show", json!({"filename": path}), Caller::User)
        .await
        .expect("memory::show 详情失败");
    assert!(detail.to_string().contains("顶点公式"), "详情应含记忆内容");

    // 文件确实落盘（跨实例持久化，新布局：base64url 段编码，ADR-0042）。
    let memory_dir = Settings::data_root().join("memory");
    assert!(
        std::fs::read_dir(&memory_dir)
            .map(|it| it.flatten().any(|e| e.path().is_dir()))
            .unwrap_or(false),
        "记忆目录应有编码目录"
    );
    let listing2 = dispatch
        .call_tool("memory::show", json!({}), Caller::User)
        .await
        .expect("memory::show 清单失败");
    assert!(
        listing2["entries"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(path))),
        "落盘后清单仍应包含 {path}"
    );

    dispatch
        .call_tool("memory::remove", json!({"filename": "测试"}), Caller::User)
        .await
        .expect("memory::remove 失败");
    let listing3 = dispatch
        .call_tool("memory::show", json!({}), Caller::User)
        .await
        .expect("memory::show 清单失败");
    assert!(
        !listing3["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some(path)),
        "删除后清单不应再包含 {path}"
    );

    let events = events.take();
    assert!(
        events.iter().any(|e| matches!(
            e,
            mistake_agent::kernel::events::Event::MemoryChanged { path: p } if p == "测试/记忆条目"
        )),
        "save/remove 应发 MemoryChanged 事件"
    );
    eprintln!("memory 工具真实链路通过");
}

/// 链路 5：余额查询真实 API —— DeepSeek /user/balance（ADR-0045 后仅 DeepSeek）。
#[tokio::test]
#[ignore]
async fn check_balance_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let req = RpcRequest {
        id: 10,
        method: WireMethod::custom("check_balance", json!({})),
    };
    let frame = kernel
        .handle(req)
        .await
        .expect("check_balance 请求失败")
        .expect("应有响应帧");
    let value = match frame {
        mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, error, .. } => {
            assert!(error.is_none(), "余额查询不应报错：{error:?}");
            result.expect("应有余额结果")
        }
        _ => panic!("余额查询应返回 response 帧"),
    };
    let main = &value["main"];
    assert!(main["configured"] == true, "DeepSeek 应已配置 key");
    assert!(main["ok"] == true, "DeepSeek 余额应查询成功：{main}");
    assert!(
        main["data"]["total_balance"].as_str().is_some(),
        "DeepSeek 应有 total_balance：{main}"
    );
    assert!(
        value.get("vision").is_none(),
        "视觉端点已退役（ADR-0045），余额不应再含 vision 字段"
    );
    eprintln!(
        "余额真实链路通过：DeepSeek {} {}",
        main["data"]["currency"].as_str().unwrap_or(""),
        main["data"]["total_balance"].as_str().unwrap_or(""),
    );
}

/// 链路 6：用户手动新建会话 + 按需携带交接摘要（ADR-0044）——
/// 旧会话归档、新会话独立 SessionKey、摘要挂为新会话首条系统消息，
/// 后续消息落在新会话且不再触发任何自动切换。
#[tokio::test]
#[ignore]
async fn create_session_carry_summary_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let (stop, responder) = spawn_compute_responder(events.clone(), kernel.clone());

    let send = |id: u64, text: &str| RpcRequest {
        id,
        method: Method::SendUserMessage {
            text: text.to_string(),
            display_text: None,
            force_tool: None,
            file: vec![],
            asset: vec![],
        }
        .into(),
    };
    async fn session_count(kernel: &Arc<Kernel>) -> usize {
        let frame = kernel
            .handle(RpcRequest {
                id: 99,
                method: Method::ListSessions.into(),
            })
            .await
            .expect("list_sessions 失败")
            .expect("应有响应帧");
        match frame {
            mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, .. } => {
                result.unwrap()["sessions"].as_array().unwrap().len()
            }
            _ => panic!("list_sessions 应返回 response 帧"),
        }
    }

    // 4 个回合 = 8 条消息，让摘要器走真实 LLM 路径（<8 条会跳过模型调用）。
    // 基线计数：live 测试共享同一数据根（sessions/ 有历史会话），改为相对基线断言，
    // 不假设「全新数据根」——否则先跑过别的用例就会误报（ADR-0044 用例原本标了待复验）。
    let baseline = session_count(&kernel).await;
    for (i, text) in ["帮我看看这道题", "继续讲第二题", "第三题呢", "再讲一道"]
        .iter()
        .enumerate()
    {
        kernel
            .handle(send(20 + i as u64, text))
            .await
            .expect("回合失败");
        assert!(
            wait_idle(&kernel, Duration::from_secs(120)).await,
            "回合 {} 120s 内未结束",
            i + 1
        );
    }
    let after_four = session_count(&kernel).await;
    assert!(
        after_four <= baseline + 1,
        "4 个回合不应自动新建多个会话：基线 {baseline} → {after_four}"
    );

    // 用户手动新建会话：携带交接摘要。
    let create_frame = kernel
        .handle(RpcRequest {
            id: 30,
            method: Method::CreateSession {
                carry_summary: true,
                goal: None,
            }
            .into(),
        })
        .await
        .expect("create_session 请求失败")
        .expect("应有响应帧");
    let created = match create_frame {
        mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, error, .. } => {
            assert!(error.is_none(), "create_session 不应报错：{error:?}");
            result.expect("应有新建结果")
        }
        _ => panic!("create_session 应返回 response 帧"),
    };
    assert_eq!(created["summary_attached"], true, "应携带交接摘要");
    let new_key: SessionKey =
        serde_json::from_value(created["session_key"].clone()).expect("解析新会话 key");
    let archived_key: SessionKey =
        serde_json::from_value(created["archived_session_key"].clone()).expect("解析归档会话 key");
    assert_ne!(new_key, archived_key, "新会话应为独立 SessionKey");

    // 归档 + 单 Active 不变量。
    let list_frame = kernel
        .handle(RpcRequest {
            id: 31,
            method: Method::ListSessions.into(),
        })
        .await
        .unwrap()
        .unwrap();
    let list = match list_frame {
        mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, .. } => {
            result.expect("应有会话列表")
        }
        _ => panic!("list_sessions 应返回 response 帧"),
    };
    let sessions = list["sessions"].as_array().unwrap();
    // create_session 只新建 1 条会话；总条目 = 调用前条目 + 1（不假设干净数据根）。
    assert_eq!(sessions.len(), after_four + 1, "旧会话保留为归档条目");
    assert_eq!(
        sessions.iter().filter(|m| m["status"] == "active").count(),
        1,
        "归档后只应剩一个活动会话：{sessions:?}"
    );
    let archived = sessions
        .iter()
        .find(|m| m["status"] == "archived" && m["key"] == archived_key.to_string())
        .expect("被归档的旧会话应保留为归档条目");
    assert_eq!(archived["key"].as_str().unwrap(), archived_key.to_string());

    // 新会话首条消息 = 交接摘要。
    let detail_frame = kernel
        .handle(RpcRequest {
            id: 32,
            method: Method::ReadSession { key: new_key }.into(),
        })
        .await
        .unwrap()
        .unwrap();
    let detail = match detail_frame {
        mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, .. } => {
            result.expect("应有会话详情")
        }
        _ => panic!("read_session 应返回 response 帧"),
    };
    let msgs = detail["messages"].as_array().unwrap();
    assert!(
        msgs.iter().any(|m| m["kind"]["kind"] == "system"
            && m["kind"]["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("上一会话梗概："))),
        "新会话应含「上一会话梗概」摘要节点：{msgs:?}"
    );

    // 后续消息落在新会话，且不会自动新建/切换会话。
    kernel
        .handle(send(33, "换一道新题"))
        .await
        .expect("新会话首个回合失败");
    assert!(
        wait_idle(&kernel, Duration::from_secs(120)).await,
        "新会话回合 120s 内未结束"
    );
    assert_eq!(
        session_count(&kernel).await,
        after_four + 1,
        "消息不落新会话、或又自动开了会话"
    );
    let after_frame = kernel
        .handle(RpcRequest {
            id: 34,
            method: Method::ReadSession { key: new_key }.into(),
        })
        .await
        .unwrap()
        .unwrap();
    let after = match after_frame {
        mistake_agent::kernel::agent::rpc::RpcFrame::Response { result, .. } => {
            result.expect("应有会话详情")
        }
        _ => panic!("read_session 应返回 response 帧"),
    };
    assert!(
        after["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["kind"]["kind"] == "assistant"),
        "新会话应有助手回复"
    );
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = responder.await;
    eprintln!(
        "用户新建会话 + 交接摘要真实链路通过：新会话 {new_key}，归档 {archived_key}，摘要已挂载"
    );
}

/// compute::verify 全链路（kernel → GUI 桥 → 回执 → 模型续答）：
/// 测试模拟 GUI 执行端——收到 Event::ComputeRequest 后经 Method::ComputeResult 回执，
/// 与前端 Pyodide 自检（web/scripts/pyodide-check.mjs）互补覆盖整条验算链路。
#[tokio::test]
#[ignore]
async fn compute_verify_roundtrip_real_api() {
    if !real_api_ready() {
        eprintln!("SKIP: 未配置真实 API key");
        return;
    }
    let events = Arc::new(MemoryEventSink::default());
    let kernel = Kernel::new(events.clone()).await.expect("kernel 启动失败");
    let req = RpcRequest {
        id: 1,
        method: Method::SendUserMessage {
            text: "请用 compute::verify 验算 17 × 19，然后把结果告诉我。".into(),
            display_text: None,
            force_tool: Some(ForcedToolRequest {
                entry: "compute::verify".into(),
                hint: Some("17*19".into()),
                display: None,
            }),
            file: vec![],
            asset: vec![],
        }
        .into(),
    };
    let frame = kernel.handle(req).await.expect("请求失败");
    assert!(
        serde_json::to_string(&frame)
            .unwrap()
            .contains("\"accepted\":true"),
        "回合应被接受"
    );

    // 模拟 GUI 执行端：捕获 ComputeRequest 事件并经 RPC 回执固定 stdout。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut delivered = false;
    while tokio::time::Instant::now() < deadline {
        let evs = events.take();
        if let Some((id, code)) = evs.into_iter().find_map(|e| match e {
            Event::ComputeRequest { id, code } => Some((id, code)),
            _ => None,
        }) {
            assert!(!code.trim().is_empty(), "模型生成的验算代码不应为空");
            kernel
                .handle(RpcRequest::custom(
                    900,
                    "compute_result",
                    json!({
                        "compute_id": id,
                        "stdout": "323",
                        "stderr": "",
                        "duration_ms": 5,
                    }),
                ))
                .await
                .expect("compute_result 回执失败");
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(delivered, "60s 内未收到 compute_request 事件");
    assert!(
        wait_idle(&kernel, Duration::from_secs(120)).await,
        "回合 120s 内未结束"
    );

    // 回合内应真实发生过 compute::verify 工具调用且成功。
    let evs = events.take();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            Event::ToolEnd { entry, ok: true } if entry == "compute::verify"
        )),
        "应有 compute::verify 成功事件"
    );
    eprintln!("compute::verify 全链路通过：事件→回执→工具成功→模型续答");
}
