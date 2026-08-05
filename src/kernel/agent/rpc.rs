//! RPC（ADR-0013 / Q15）：stdio JSONL，JSON-RPC 2.0 风格（id + notification）。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::kernel::agent::cache::CacheTracker;
use crate::kernel::agent::dispatch::Dispatch;
use crate::kernel::agent::loop_mod::{AgentLoop, TurnInput, TurnOutcome};
use crate::kernel::agent::session::{
    Interrupt, InterruptBus, LlmSummarizer, LlmTurnDecider, SessionKey, SessionScheduler,
    SessionStatus, SessionSwitch, SystemClock,
};
use crate::kernel::audit::{AuditRecord, Auditor};
use crate::kernel::contract::{CallerPolicy, full_to_wire};
use crate::kernel::events::{Event, EventSink};
use crate::kernel::logger::{Logger, LoggerHandle};
use crate::kernel::message::{Message, MessageId};
use crate::kernel::plugin::compute::BridgeCompute;
use crate::kernel::plugin::memory::{FileMemoryService, InMemoryMemory};
use crate::kernel::plugin::model::{LiveSettingsModelService, RoutingModelService};
use crate::kernel::plugin::services::{
    AbortSignal, ComputeHandle, MemoryHandle, MemoryService, ModelHandle, ModelKind, ModelRequest,
    ModelService, ServiceHandles, SessionStore, StorageHandle,
};
use crate::kernel::plugin::storage::{AnyStorage, FileStorage};
use crate::kernel::registry::Registry;
use crate::kernel::settings::Settings;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    SendUserMessage {
        text: String,
        /// 显式工具调用：强制 LLM 首轮调用指定工具（不绕过 LLM）。
        #[serde(default)]
        force_tool: Option<ForcedToolRequest>,
        /// 暂存文件路径列表（mistake-agent- 前缀临时路径）：模型读图/判分时作为 file 参数。
        #[serde(default)]
        file: Vec<String>,
        /// 持久附件列表（数据根目录 uploads/ 副本）：落进消息文本供前端展示。
        #[serde(default)]
        asset: Vec<AttachmentInfo>,
    },
    TriggerCommand {
        entry: String,
        params: Value,
    },
    EditMessage {
        message_id: MessageId,
        text: String,
    },
    SwitchBranch {
        message_id: MessageId,
    },
    Abort,
    GetState,
    GetSettings,
    SetSettings {
        patch: crate::kernel::settings::SettingsPatch,
    },
    /// GUI 验算执行端回执（compute 桥接）。
    ComputeResult {
        /// 与 RPC 请求帧顶层 id 区分，前端按 compute_id 回执。
        #[serde(rename = "compute_id")]
        id: u64,
        stdout: String,
        stderr: String,
        duration_ms: u64,
    },
    /// 会话列表（GUI 会话历史页）。
    ListSessions,
    /// 读取指定会话完整消息树（GUI 历史浏览/分支回放）。
    ReadSession {
        key: SessionKey,
    },
    /// 用户可调工具/命令清单（GUI 工具面板）。
    ListTools,
    /// 连通性自检：主/视觉模型轻量调用（OOBE 引导"测试连接"用）。
    /// 可携带表单中的临时 api_key（不落盘，仅本次请求生效）；model 取 "main"/"vision"。
    TestConnection {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    /// 账户余额查询：DeepSeek /user/balance + SiliconFlow /user/info（只读，不落盘 key）。
    CheckBalance,
    /// 聊天上下文缓存命中统计（主模型回合调用累计，按会话 + 全局）。
    GetCacheStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    #[serde(flatten)]
    pub method: Method,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcFrame {
    Response {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
    Event {
        event: Event,
    },
}

/// 显式工具调用请求：entry 为内部全名（namespace::tool），hint 为用户输入的可选参数文本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForcedToolRequest {
    pub entry: String,
    #[serde(default)]
    pub hint: Option<String>,
    /// 前端原始展示文本（如「翻看记忆：数学/向量组…」）；缺省时 kernel 按 title＋hint 兜底，
    /// 落盘到 user 消息的 display_text，重开会话后渲染仍友好。
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub path: String,
    pub name: String,
}

/// 落盘回合新增消息：过滤 `session::switch` 控制消息（ADR-0034，不落会话树），
/// 并把其直接子消息的父链重接到切换前的最后一条消息；跳过压缩摘要
/// （由 splice_compaction 单独接入）。返回最后落盘消息 id（None = 无可落盘消息）。
async fn persist_turn_messages(
    store: &Arc<dyn SessionStore>,
    key: &SessionKey,
    messages: &[Message],
    skip_id: Option<MessageId>,
) -> Result<Option<MessageId>, String> {
    let mut last_kept: Option<MessageId> = None;
    let mut skipped_switch: Option<MessageId> = None;
    for msg in messages {
        if Some(msg.id) == skip_id {
            continue;
        }
        if msg.is_switch_tool_call() {
            if last_kept.is_none() {
                last_kept = msg.parent_id;
            }
            skipped_switch = Some(msg.id);
            continue;
        }
        let mut m = msg.clone();
        if skipped_switch.is_some_and(|sid| m.parent_id == Some(sid))
            && let Some(anchor) = last_kept
        {
            m.parent_id = Some(anchor);
        }
        store
            .append_message(key, &m)
            .await
            .map_err(|e| e.to_string())?;
        last_kept = Some(m.id);
    }
    Ok(last_kept)
}

struct TurnHandle {
    key: SessionKey,
    signal: AbortSignal,
}

struct KernelState {
    turn: Option<TurnHandle>,
}

pub struct Kernel {
    registry: Arc<Registry>,
    dispatch: Arc<Dispatch>,
    loop_engine: Arc<AgentLoop>,
    scheduler: Arc<SessionScheduler>,
    store: Arc<dyn SessionStore>,
    auditor: Auditor,
    events: Arc<dyn EventSink>,
    compute: Arc<BridgeCompute>,
    settings: Arc<std::sync::RwLock<Settings>>,
    main_service: Arc<LiveSettingsModelService>,
    vision_service: Arc<LiveSettingsModelService>,
    state: Arc<Mutex<KernelState>>,
    cache: Arc<CacheTracker>,
}

impl Kernel {
    /// 组装内核（M1 全内存 + 真模型适配器）。
    pub async fn new(events: Arc<dyn EventSink>) -> Result<Arc<Self>, String> {
        let settings = Arc::new(std::sync::RwLock::new(Settings::load()?));
        let data_root = Settings::data_root();
        // 数据根目录一次性初始化（子目录 + AGENTS.md 模板，幂等）。
        crate::kernel::bootstrap::init_data_root(&data_root)?;
        Logger::init(
            settings.read().expect("settings poisoned").log_level,
            &data_root.join("logs"),
        )?;
        let logger: LoggerHandle = Arc::new(Logger);

        let storage = Arc::new(match FileStorage::open(&data_root) {
            Ok(file) => AnyStorage::File(file),
            Err(e) => {
                eprintln!("[kernel] 文件存储打开失败，回退内存存储：{e}");
                AnyStorage::Mem(crate::kernel::plugin::storage::MemoryStorage::new())
            }
        });
        let memory: Arc<dyn MemoryService> = match FileMemoryService::open_default() {
            Ok(file_memory) => Arc::new(file_memory),
            Err(e) => {
                eprintln!("[kernel] 记忆目录打开失败，回退内存记忆：{e}");
                Arc::new(InMemoryMemory::new())
            }
        };
        let compute = Arc::new(BridgeCompute::new(events.clone()));
        let main_service = Arc::new(LiveSettingsModelService::new(
            settings.clone(),
            ModelKind::Main,
        ));
        let vision_service = Arc::new(LiveSettingsModelService::new(
            settings.clone(),
            ModelKind::Vision,
        ));
        let cache = Arc::new(CacheTracker::default());

        let auditor = Auditor::new(storage.clone());
        let router = Arc::new(RoutingModelService::new(
            main_service.clone() as Arc<dyn crate::kernel::plugin::services::ModelService>,
            vision_service.clone() as Arc<dyn crate::kernel::plugin::services::ModelService>,
        ));
        let handles = ServiceHandles::default()
            .with_storage(StorageHandle::new(storage.clone()))
            .with_memory(MemoryHandle::with_observability(
                memory.clone(),
                events.clone(),
                auditor.clone(),
            ))
            .with_compute(ComputeHandle::new(compute.clone()))
            .with_model(ModelHandle::new(
                router,
                std::time::Duration::from_secs(180),
                auditor.clone(),
            ));

        let registry = Arc::new(Registry::new(handles, logger));
        for desc in crate::kernel::plugin::builtin_kernel_plugins() {
            registry
                .register_kernel_plugin(desc)
                .map_err(|e| format!("内核插件注册失败：{e}"))?;
        }
        for desc in crate::plugin::builtin_plugins() {
            registry
                .register_plugin(desc)
                .map_err(|e| format!("插件注册失败：{e}"))?;
        }

        let dispatch = Arc::new(Dispatch::new(
            registry.clone(),
            auditor.clone(),
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(10 * 60),
            events.clone(),
        ));
        // 中断总线必须由 scheduler 与 loop 共享：scheduler 发环境变更，loop 回合边界消费。
        let interrupt_bus = InterruptBus::new();
        let scheduler = Arc::new(SessionScheduler::new(
            storage.clone(),
            Arc::new(LlmTurnDecider::new(main_service.clone())),
            Arc::new(SystemClock),
            Arc::new(LlmSummarizer::new(main_service.clone())),
            interrupt_bus.clone(),
        ));
        let loop_engine = Arc::new(AgentLoop::new(
            main_service.clone(),
            dispatch.clone(),
            auditor.clone(),
            events.clone(),
            Arc::new(LlmSummarizer::new(main_service.clone())),
            interrupt_bus.clone(),
            Some(scheduler.clone() as Arc<dyn SessionSwitch>),
        ));

        Ok(Arc::new(Self {
            registry,
            dispatch,
            loop_engine,
            scheduler,
            store: storage.clone(),
            auditor,
            events,
            compute,
            settings,
            main_service,
            vision_service,
            state: Arc::new(Mutex::new(KernelState { turn: None })),
            cache,
        }))
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn dispatch(&self) -> Arc<Dispatch> {
        self.dispatch.clone()
    }

    async fn active_session_key(&self) -> Result<SessionKey, RpcError> {
        let metas = self
            .store
            .list_sessions()
            .await
            .map_err(|e| RpcError::new("storage_error", e.to_string()))?;
        metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .map(|m| m.key)
            .ok_or_else(|| RpcError::new("no_active_session", "没有活动会话"))
    }

    /// 当前是否有回合在跑（GUI 关闭收尾时轮询用）。
    pub async fn is_idle(&self) -> bool {
        self.state.lock().await.turn.is_none()
    }

    /// 处理一个请求；返回需要写回 GUI 的响应帧（事件经 EventSink 另发）。
    pub async fn handle(&self, request: RpcRequest) -> Result<Option<RpcFrame>, RpcError> {
        match request.method {
            Method::SendUserMessage {
                text,
                force_tool,
                file,
                asset,
            } => {
                {
                    let state = self.state.lock().await;
                    if state.turn.is_some() {
                        return Err(RpcError::new(
                            "turn_in_progress",
                            "当前有回合在跑，请先停止再发送新消息",
                        ));
                    }
                }
                // 显式工具调用：构造"强制调用"用户消息并让 loop 首轮带 tool_choice。
                let mut user_text = text.clone();
                let mut display_text: Option<String> = None;
                let mut forced_wire: Option<String> = None;
                if let Some(ft) = force_tool {
                    let entry = self
                        .registry
                        .ensure_tool(&ft.entry)
                        .map_err(|e| RpcError::new("unknown_tool", e.to_string()))?;
                    if entry.policy == CallerPolicy::UserOnly {
                        return Err(RpcError::new(
                            "forbidden_tool",
                            "该工具仅用户可调，不能被模型强制调用",
                        ));
                    }
                    let hint = ft.hint.as_deref().unwrap_or("").trim();
                    user_text = if hint.is_empty() {
                        format!("请调用工具 {} 处理当前请求。", ft.entry)
                    } else {
                        format!("请调用工具 {} 处理：{}", ft.entry, hint)
                    };
                    // 展示文本：优先用前端原始展示（display），否则按 title＋hint 兜底。
                    display_text =
                        ft.display
                            .clone()
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| {
                                self.registry.entry_title(&ft.entry).map(|title| {
                                    if hint.is_empty() {
                                        title
                                    } else {
                                        format!("{title}：{hint}")
                                    }
                                })
                            });
                    forced_wire = Some(full_to_wire(&ft.entry));
                }
                // 附件信息追加进模型指令（展示文本保持用户原文，不暴露路径）：
                // 暂存文件路径供模型作 file 参数，持久副本标记供前端展示附件。
                for f in &file {
                    user_text.push_str(&format!("\n暂存文件：{f}"));
                    display_text.get_or_insert_with(|| text.clone());
                }
                for a in &asset {
                    user_text.push_str(&format!("\n附件：{}|{}", a.path, a.name));
                    display_text.get_or_insert_with(|| text.clone());
                }
                // 会话调度（守卫/摘要可能调用 LLM 数十秒）在锁外执行，
                // 避免阻塞 abort/get_state 等请求。
                let ctx = self
                    .scheduler
                    .on_new_message_with_display(&user_text, display_text.as_deref())
                    .await
                    .map_err(|e| RpcError::new("scheduler_error", e.to_string()))?;
                let signal = AbortSignal::new();
                let key = ctx.session_key;
                let tools = self.registry.model_tools();
                let messages = ctx.messages;
                let loop_engine = self.loop_engine.clone();
                let scheduler = self.scheduler.clone();
                let store = self.store.clone();
                let events = self.events.clone();
                let auditor = self.auditor.clone();
                let cache = self.cache.clone();
                let state_for_task = self.state.clone();
                let mut state = self.state.lock().await;
                if state.turn.is_some() {
                    // 并发竞态兜底：另一请求已登记回合。
                    return Err(RpcError::new(
                        "turn_in_progress",
                        "当前有回合在跑，请先停止再发送新消息",
                    ));
                }
                state.turn = Some(TurnHandle {
                    key,
                    signal: signal.clone(),
                });
                drop(state);

                tokio::spawn(async move {
                    let input = TurnInput {
                        messages,
                        tools,
                        signal,
                        turn_budget: std::time::Duration::from_secs(10 * 60),
                        forced_tool: forced_wire,
                    };
                    let outcome: Result<TurnOutcome, _> = loop_engine.run_turn(input).await;
                    match outcome {
                        Ok(outcome) => {
                            let compaction = outcome.compaction.clone();
                            // 回合内经 session::switch 切换后，后半段消息归新会话。
                            let persist_key = outcome.session_key.unwrap_or(key);
                            let skip_summary = compaction.as_ref().map(|c| c.summary.id);
                            let persisted_last = match persist_turn_messages(
                                &store,
                                &persist_key,
                                &outcome.messages,
                                skip_summary,
                            )
                            .await
                            {
                                Ok(last) => last,
                                Err(e) => {
                                    events.emit(Event::Error {
                                        message: format!("消息落盘失败：{e}"),
                                    });
                                    None
                                }
                            };
                            if let Some(info) = &compaction {
                                if let Err(e) = store
                                    .splice_compaction(&persist_key, &info.summary, info.tail_start)
                                    .await
                                {
                                    events.emit(Event::Error {
                                        message: format!("压缩摘要接入失败：{e}"),
                                    });
                                }
                                events.emit(Event::Compaction {
                                    session: persist_key,
                                });
                                auditor.record(AuditRecord::Compaction {
                                    session: persist_key.to_string(),
                                    summarized: info.summarized,
                                });
                                scheduler.interrupt_bus().send(Interrupt::CompactionDone {
                                    session: persist_key,
                                });
                            }
                            // 活跃路径推进到回合末条（消息树分支语义）。
                            let next_active =
                                compaction.as_ref().map(|c| c.tail_end).or(persisted_last);
                            if let Some(next) = next_active
                                && let Err(e) =
                                    store.set_active_path(&persist_key, Some(next)).await
                            {
                                events.emit(Event::Error {
                                    message: format!("活跃路径推进失败：{e}"),
                                });
                            }
                            if let Err(e) =
                                scheduler.on_turn_end(&persist_key, &outcome.messages).await
                            {
                                events.emit(Event::Error {
                                    message: format!("回合收尾失败：{e}"),
                                });
                            }
                            if let Some(usage) = &outcome.usage {
                                cache.record_main(&persist_key, usage);
                                // 实时推送：前端收到事件即更新，无需再查一次（可能读到旧值）。
                                events.emit(Event::CacheStatsUpdated {
                                    stats: cache.snapshot(Some(persist_key)),
                                });
                            }
                            auditor.record(AuditRecord::Lifecycle {
                                phase: "turn_finished".into(),
                            });
                        }
                        Err(e) => {
                            events.emit(Event::TurnEnd {
                                stop_reason: crate::kernel::agent::loop_mod::StopReason::Failed,
                            });
                            events.emit(Event::Error {
                                message: format!("回合失败：{e}"),
                            });
                        }
                    }
                    // 回合结束：清除 turn 句柄（abort 在结束后无操作）。
                    let mut st = state_for_task.lock().await;
                    if st.turn.as_ref().is_some_and(|t| t.key == key) {
                        st.turn = None;
                    }
                });

                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({"accepted": true})),
                    error: None,
                }))
            }
            Method::TriggerCommand { entry, params } => {
                let result = self.dispatch.call_command(&entry, params).await;
                let frame = match result {
                    Ok(v) => RpcFrame::Response {
                        id: request.id,
                        result: Some(v),
                        error: None,
                    },
                    Err(e) => RpcFrame::Response {
                        id: request.id,
                        result: None,
                        error: Some(RpcError::new("tool_error", e.message)),
                    },
                };
                Ok(Some(frame))
            }
            Method::Abort => {
                let state = self.state.lock().await;
                let aborted = if let Some(turn) = &state.turn {
                    turn.signal.cancel();
                    true
                } else {
                    false
                };
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({"aborted": aborted})),
                    error: None,
                }))
            }
            Method::GetState => {
                let state = self.state.lock().await;
                let (status, session_key) = match &state.turn {
                    Some(t) => ("busy", Some(t.key.to_string())),
                    None => ("idle", None),
                };
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({"status": status, "session_key": session_key})),
                    error: None,
                }))
            }
            Method::ListSessions => {
                let metas = self
                    .store
                    .list_sessions()
                    .await
                    .map_err(|e| RpcError::new("storage_error", e.to_string()))?;
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({
                        "sessions": serde_json::to_value(&metas).unwrap_or_default(),
                    })),
                    error: None,
                }))
            }
            Method::ReadSession { key } => {
                let meta = self
                    .store
                    .get_session(&key)
                    .await
                    .map_err(|e| RpcError::new("storage_error", e.to_string()))?;
                let messages = self
                    .store
                    .read_all(&key)
                    .await
                    .map_err(|e| RpcError::new("storage_error", e.to_string()))?;
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({
                        "meta": serde_json::to_value(&meta).unwrap_or_default(),
                        "messages": serde_json::to_value(&messages).unwrap_or_default(),
                    })),
                    error: None,
                }))
            }
            Method::ListTools => {
                let tools = self.registry.user_entries();
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({ "tools": tools })),
                    error: None,
                }))
            }
            Method::TestConnection { api_key, model } => {
                let started = std::time::Instant::now();
                let is_vision = model.as_deref() == Some("vision");
                let model_req = ModelRequest {
                    model: ModelKind::Main,
                    messages: vec![Message::user("回复：ok")],
                    tools: None,
                    reasoning_effort: Some("none".into()),
                    tool_choice: None,
                    response_format: None,
                };
                let result = if let Some(key) = api_key
                    && !key.trim().is_empty()
                {
                    // 临时 key：仅本次请求生效（不落盘、不改 settings）。
                    let snapshot = self.settings.read().expect("settings poisoned").clone();
                    let mut model_cfg = if is_vision {
                        snapshot.vision_model.clone()
                    } else {
                        snapshot.main_model.clone()
                    };
                    model_cfg.api_key = key.trim().to_string();
                    let temp_settings = if is_vision {
                        crate::kernel::settings::Settings {
                            log_level: snapshot.log_level,
                            main_model: snapshot.main_model.clone(),
                            vision_model: model_cfg,
                        }
                    } else {
                        crate::kernel::settings::Settings {
                            log_level: snapshot.log_level,
                            main_model: model_cfg,
                            vision_model: snapshot.vision_model.clone(),
                        }
                    };
                    if is_vision {
                        crate::kernel::plugin::model::build_vision_service(&temp_settings)
                            .complete(&model_req, &AbortSignal::new())
                            .await
                    } else {
                        crate::kernel::plugin::model::build_main_service(&temp_settings)
                            .complete(&model_req, &AbortSignal::new())
                            .await
                    }
                } else if is_vision {
                    self.vision_service
                        .complete(&model_req, &AbortSignal::new())
                        .await
                } else {
                    self.main_service
                        .complete(&model_req, &AbortSignal::new())
                        .await
                };
                match result {
                    Ok(_) => Ok(Some(RpcFrame::Response {
                        id: request.id,
                        result: Some(json!({
                            "ok": true,
                            "latency_ms": started.elapsed().as_millis() as u64,
                        })),
                        error: None,
                    })),
                    Err(e) => Ok(Some(RpcFrame::Response {
                        id: request.id,
                        result: None,
                        error: Some(RpcError::new("connection_failed", e.to_string())),
                    })),
                }
            }
            Method::CheckBalance => {
                let settings = self.settings.read().expect("settings poisoned").clone();
                let report = crate::kernel::agent::balance::check_balance(&settings).await;
                self.auditor.record(AuditRecord::BalanceChecked {
                    main_ok: report.main.ok,
                    vision_ok: report.vision.ok,
                });
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(
                        serde_json::to_value(&report).unwrap_or_else(|_| serde_json::json!({})),
                    ),
                    error: None,
                }))
            }
            Method::GetCacheStats => {
                let active = self.active_session_key().await.ok();
                let snapshot = self.cache.snapshot(active);
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(snapshot),
                    error: None,
                }))
            }
            Method::EditMessage { message_id, text } => {
                let key = self.active_session_key().await?;
                let path = self
                    .store
                    .derive_branch(&key, message_id, &text)
                    .await
                    .map_err(|e| RpcError::new("branch_error", e.to_string()))?;
                let branch_id = path.last().map(|m| m.id).unwrap_or(message_id);
                self.auditor.record(AuditRecord::MessageEdited {
                    message_id,
                    branch_id,
                });
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({
                        "session_key": key,
                        "messages": serde_json::to_value(&path).unwrap_or_default(),
                    })),
                    error: None,
                }))
            }
            Method::SwitchBranch { message_id } => {
                let key = self.active_session_key().await?;
                let path = self
                    .store
                    .switch_branch(&key, message_id)
                    .await
                    .map_err(|e| RpcError::new("branch_error", e.to_string()))?;
                self.auditor
                    .record(AuditRecord::BranchSwitched { message_id });
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({
                        "session_key": key,
                        "messages": serde_json::to_value(&path).unwrap_or_default(),
                    })),
                    error: None,
                }))
            }
            Method::GetSettings => {
                let view = self
                    .settings
                    .read()
                    .expect("settings poisoned")
                    .public_view();
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(view),
                    error: None,
                }))
            }
            Method::SetSettings { patch } => {
                let view = {
                    let mut settings = self.settings.write().expect("settings poisoned");
                    settings
                        .apply_patch(&patch)
                        .map_err(|e| RpcError::new("invalid_settings", e))?;
                    settings
                        .save()
                        .map_err(|e| RpcError::new("save_failed", e))?;
                    if let Some(level) = patch.log_level {
                        Logger::set_level(level);
                    }
                    settings.public_view()
                };
                log::info!(
                    "设置已保存并热更新：main_key_set={} vision_key_set={}",
                    view["main_model"]["key_set"],
                    view["vision_model"]["key_set"]
                );
                // 模型配置热更新：下一次模型调用即用新端点/模型/key。
                self.main_service.refresh();
                self.vision_service.refresh();
                self.scheduler
                    .interrupt_bus()
                    .send(Interrupt::SettingsChanged);
                self.auditor.record(AuditRecord::SettingsChanged);
                // OOBE 完成路径兜底：目录与 AGENTS.md 初始化（幂等，Kernel::new 已做过一次）。
                crate::kernel::bootstrap::init_data_root(&crate::kernel::settings::Settings::data_root())
                    .map_err(|e| RpcError::new("bootstrap_failed", e))?;
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(view),
                    error: None,
                }))
            }
            Method::ComputeResult {
                id,
                stdout,
                stderr,
                duration_ms,
            } => {
                let delivered = self.compute.deliver(
                    id,
                    crate::kernel::plugin::services::ComputeResult {
                        stdout,
                        stderr,
                        duration_ms,
                    },
                );
                Ok(Some(RpcFrame::Response {
                    id: request.id,
                    result: Some(json!({ "delivered": delivered })),
                    error: None,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::agent::session::SessionMeta;
    use crate::kernel::plugin::storage::MemoryStorage;

    #[tokio::test]
    async fn switch_tool_call_not_persisted_and_children_reparented() {
        let store: Arc<dyn SessionStore> = Arc::new(MemoryStorage::new());
        let key = SessionKey::new();
        store
            .create_session(&key, &SessionMeta::new(key))
            .await
            .unwrap();
        let user = Message::user("帮我批改数学作业");
        store.append_message(&key, &user).await.unwrap();

        let mut switch = Message::tool_call(
            "session::switch",
            json!({"goal": "批改英语作业"}),
            Ok(json!({"switched": true})),
        );
        switch.parent_id = Some(user.id);
        let mut answer = Message::assistant("好的，先切换到英语作业");
        answer.parent_id = Some(switch.id);
        let answer_id = answer.id;

        let last = persist_turn_messages(&store, &key, &[switch, answer], None)
            .await
            .unwrap();
        assert_eq!(last, Some(answer_id));

        let path = store.read_path(&key).await.unwrap();
        assert_eq!(path.len(), 2, "切换控制消息不应落盘");
        assert!(!path[1].is_switch_tool_call());
        assert_eq!(
            path[1].parent_id,
            Some(user.id),
            "子消息父链应重接到切换前最后一条"
        );
    }
}
