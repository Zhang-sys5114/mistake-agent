//! RPC（ADR-0013 / Q15）：stdio JSONL，JSON-RPC 2.0 风格（id + notification）。

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::kernel::agent::cache::CacheTracker;
use crate::kernel::agent::dispatch::Dispatch;
use crate::kernel::agent::loop_mod::{AgentLoop, SystemPromptProvider, TurnInput, TurnOutcome};
use crate::kernel::agent::session::{
    Interrupt, InterruptBus, LlmSummarizer, LlmTurnDecider, SessionKey, SessionScheduler,
    SessionStatus, SessionSwitch, SystemClock, scope_session_context,
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
use crate::kernel::registry::{KernelDescriptor, PluginDescriptor, Registry};
use crate::kernel::settings::Settings;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    /// 通用子集：新 Agent 直接可用，不依赖使用方业务。
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
    /// 会话列表（GUI 会话历史页）。
    ListSessions,
    /// 读取指定会话完整消息树（GUI 历史浏览/分支回放）。
    ReadSession {
        key: SessionKey,
    },
    /// 用户可调工具/命令清单（GUI 工具面板）。
    ListTools,
}

/// 自定义方法兜底：未知 method 名连同 params 与其余字段一起交给 `RpcExtension`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMethod {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// 前端可能直接把方法字段平铺在请求顶层（如 compute_result 的 compute_id），
    /// 这里保留全部多余字段，供扩展实现按原 wire 形状解析。
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// RPC 请求承载：通用子集优先匹配，其余走自定义兜底（保持既有 wire 兼容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WireMethod {
    Generic(Method),
    Custom(CustomMethod),
}

impl From<Method> for WireMethod {
    fn from(method: Method) -> Self {
        Self::Generic(method)
    }
}

impl WireMethod {
    pub fn custom(method: impl Into<String>, params: Value) -> Self {
        Self::Custom(CustomMethod {
            method: method.into(),
            params,
            extra: BTreeMap::new(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    #[serde(flatten)]
    pub method: WireMethod,
}

impl RpcRequest {
    pub fn custom(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            id,
            method: WireMethod::custom(method, params),
        }
    }
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

/// 自定义 RPC 扩展：通用子集之外的方法由使用方注册，返回 `Ok(None)` 表示不处理。
#[async_trait]
pub trait RpcExtension: Send + Sync {
    async fn handle(&self, method: &str, params: Value) -> Result<Option<Value>, RpcError>;
}

/// 把自定义请求的 `params` 与平铺多余字段合并，兼容新旧两种 wire 形状。
fn custom_params(c: &CustomMethod) -> Value {
    if c.extra.is_empty() {
        return c.params.clone();
    }
    let mut map = match &c.params {
        Value::Object(obj) => obj.clone(),
        Value::Null => serde_json::Map::new(),
        other => {
            let mut obj = serde_json::Map::new();
            obj.insert("params".into(), other.clone());
            obj
        }
    };
    for (k, v) in &c.extra {
        map.insert(k.clone(), v.clone());
    }
    Value::Object(map)
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
    state: Arc<Mutex<KernelState>>,
    cache: Arc<CacheTracker>,
    extensions: Vec<Arc<dyn RpcExtension>>,
}

/// 通用 Kernel 装配入口：事件、句柄、插件、系统提示与 RPC 扩展都经 builder 注入，
/// 应用专属服务（FileStorage/LiveSettingsModelService/AppRpc…）由使用方在 `Kernel::new`
/// 或自己的装配函数中构造后传入。
pub struct KernelBuilder {
    events: Arc<dyn EventSink>,
    system_prompt: SystemPromptProvider,
    handles: ServiceHandles,
    store: Option<Arc<dyn SessionStore>>,
    main_model: Option<Arc<dyn ModelService>>,
    auditor: Option<Auditor>,
    cache: Option<Arc<CacheTracker>>,
    interrupt_bus: InterruptBus,
    kernel_plugins: Vec<KernelDescriptor>,
    user_plugins: Vec<PluginDescriptor>,
    extensions: Vec<Arc<dyn RpcExtension>>,
}

impl Default for KernelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelBuilder {
    pub fn new() -> Self {
        Self {
            events: Arc::new(crate::kernel::events::MemoryEventSink::default()),
            system_prompt: Arc::new(String::new),
            handles: ServiceHandles::default(),
            store: None,
            main_model: None,
            auditor: None,
            cache: None,
            interrupt_bus: InterruptBus::new(),
            kernel_plugins: Vec::new(),
            user_plugins: Vec::new(),
            extensions: Vec::new(),
        }
    }

    pub fn event_sink(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = events;
        self
    }

    pub fn system_prompt(mut self, provider: SystemPromptProvider) -> Self {
        self.system_prompt = provider;
        self
    }

    pub fn service_handles(mut self, handles: ServiceHandles) -> Self {
        self.handles = handles;
        self
    }

    pub fn session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn main_model(mut self, model: Arc<dyn ModelService>) -> Self {
        self.main_model = Some(model);
        self
    }

    pub fn auditor(mut self, auditor: Auditor) -> Self {
        self.auditor = Some(auditor);
        self
    }

    pub fn cache(mut self, cache: Arc<CacheTracker>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn interrupt_bus(mut self, bus: InterruptBus) -> Self {
        self.interrupt_bus = bus;
        self
    }

    pub fn register_kernel_plugin(mut self, desc: KernelDescriptor) -> Self {
        self.kernel_plugins.push(desc);
        self
    }

    pub fn register_plugin(mut self, desc: PluginDescriptor) -> Self {
        self.user_plugins.push(desc);
        self
    }

    pub fn register_kernel_plugins(mut self, descs: Vec<KernelDescriptor>) -> Self {
        self.kernel_plugins.extend(descs);
        self
    }

    pub fn register_plugins(mut self, descs: Vec<PluginDescriptor>) -> Self {
        self.user_plugins.extend(descs);
        self
    }

    pub fn extension(mut self, ext: Arc<dyn RpcExtension>) -> Self {
        self.extensions.push(ext);
        self
    }

    pub async fn build(self) -> Result<Arc<Kernel>, String> {
        let store = self
            .store
            .ok_or_else(|| "KernelBuilder 缺少 session_store".to_string())?;
        let main_model = self
            .main_model
            .ok_or_else(|| "KernelBuilder 缺少 main_model".to_string())?;
        let auditor = self
            .auditor
            .ok_or_else(|| "KernelBuilder 缺少 auditor".to_string())?;
        let cache = self.cache.unwrap_or_default();

        let logger: LoggerHandle = Arc::new(Logger);
        let registry = Arc::new(Registry::new(self.handles, logger));
        for desc in self.kernel_plugins {
            registry
                .register_kernel_plugin(desc)
                .map_err(|e| format!("内核插件注册失败：{e}"))?;
        }
        for desc in self.user_plugins {
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
            self.events.clone(),
        ));
        // 中断总线必须由 scheduler 与 loop 共享：scheduler 发环境变更，loop 回合边界消费。
        let scheduler = Arc::new(SessionScheduler::new(
            store.clone(),
            Arc::new(LlmTurnDecider::new(main_model.clone())),
            Arc::new(SystemClock),
            Arc::new(LlmSummarizer::new(main_model.clone())),
            self.interrupt_bus.clone(),
        ));
        let loop_model = main_model.clone();
        let loop_engine = Arc::new(AgentLoop::new(
            loop_model,
            dispatch.clone(),
            auditor.clone(),
            self.events.clone(),
            Arc::new(LlmSummarizer::new(main_model)),
            self.interrupt_bus,
            self.system_prompt,
            Some(scheduler.clone() as Arc<dyn SessionSwitch>),
        ));

        Ok(Arc::new(Kernel {
            registry,
            dispatch,
            loop_engine,
            scheduler,
            store,
            auditor,
            events: self.events,
            state: Arc::new(Mutex::new(KernelState { turn: None })),
            cache,
            extensions: self.extensions,
        }))
    }
}

/// mistake-agent 应用专属 RPC 扩展：settings/balance/cache/compute 方法走 custom 兜底，
/// 不占通用 `Method` 子集（M1 解耦，提取 so-lite-agent 时留在使用方）。
struct AppRpc {
    settings: Arc<std::sync::RwLock<Settings>>,
    store: Arc<dyn SessionStore>,
    main_service: Arc<LiveSettingsModelService>,
    vision_service: Arc<LiveSettingsModelService>,
    compute: Arc<BridgeCompute>,
    cache: Arc<CacheTracker>,
    interrupt_bus: InterruptBus,
    auditor: Auditor,
}

#[async_trait]
impl RpcExtension for AppRpc {
    async fn handle(&self, method: &str, params: Value) -> Result<Option<Value>, RpcError> {
        match method {
            "get_settings" => Ok(Some(
                self.settings
                    .read()
                    .expect("settings poisoned")
                    .public_view(),
            )),
            "set_settings" => {
                let patch: crate::kernel::settings::SettingsPatch = serde_json::from_value(
                    params.get("patch").cloned().unwrap_or(Value::Null),
                )
                .map_err(|e| RpcError::new("invalid_settings", e.to_string()))?;
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
                self.main_service.refresh();
                self.vision_service.refresh();
                self.interrupt_bus.send(Interrupt::ConfigChanged);
                self.auditor.record(AuditRecord::SettingsChanged);
                crate::kernel::bootstrap::init_data_root(
                    &crate::kernel::settings::Settings::data_root(),
                )
                .map_err(|e| RpcError::new("bootstrap_failed", e))?;
                Ok(Some(view))
            }
            "compute_result" => {
                let id = params
                    .get("compute_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| RpcError::new("invalid_params", "缺少 compute_id"))?;
                let result = crate::kernel::plugin::services::ComputeResult {
                    stdout: params
                        .get("stdout")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    stderr: params
                        .get("stderr")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    duration_ms: params.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
                };
                let delivered = self.compute.deliver(id, result);
                Ok(Some(json!({ "delivered": delivered })))
            }
            "test_connection" => {
                let started = std::time::Instant::now();
                let is_vision = params.get("model").and_then(Value::as_str) == Some("vision");
                let model_req = ModelRequest {
                    model: ModelKind::Main,
                    messages: vec![Message::user("回复：ok")],
                    tools: None,
                    reasoning_effort: Some("none".into()),
                    tool_choice: None,
                    response_format: None,
                };
                let api_key = params.get("api_key").and_then(Value::as_str);
                let result = if let Some(key) = api_key
                    && !key.trim().is_empty()
                {
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
                    Ok(_) => Ok(Some(json!({
                        "ok": true,
                        "latency_ms": started.elapsed().as_millis() as u64,
                    }))),
                    Err(e) => Err(RpcError::new("connection_failed", e.to_string())),
                }
            }
            "check_balance" => {
                let settings = self.settings.read().expect("settings poisoned").clone();
                let report = crate::kernel::agent::balance::check_balance(&settings).await;
                self.auditor.record(AuditRecord::BalanceChecked {
                    main_ok: report.main.ok,
                    vision_ok: report.vision.ok,
                });
                Ok(Some(
                    serde_json::to_value(&report).unwrap_or_else(|_| serde_json::json!({})),
                ))
            }
            "get_cache_stats" => {
                let metas = self
                    .store
                    .list_sessions()
                    .await
                    .map_err(|e| RpcError::new("storage_error", e.to_string()))?;
                let active = metas
                    .iter()
                    .find(|m| m.status == SessionStatus::Active)
                    .map(|m| m.key);
                Ok(Some(self.cache.snapshot(active)))
            }
            _ => Ok(None),
        }
    }
}

impl Kernel {
    /// mistake-agent 便捷装配：应用专属服务 + 通用 KernelBuilder。
    pub async fn new(events: Arc<dyn EventSink>) -> Result<Arc<Self>, String> {
        let settings = Arc::new(std::sync::RwLock::new(Settings::load()?));
        let data_root = Settings::data_root();
        // 数据根目录一次性初始化（子目录 + AGENTS.md 模板，幂等）。
        crate::kernel::bootstrap::init_data_root(&data_root)?;
        Logger::init(
            settings.read().expect("settings poisoned").log_level,
            &data_root.join("logs"),
        )?;
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

        let interrupt_bus = InterruptBus::new();
        let app_rpc = AppRpc {
            settings: settings.clone(),
            store: storage.clone(),
            main_service: main_service.clone(),
            vision_service: vision_service.clone(),
            compute: compute.clone(),
            cache: cache.clone(),
            interrupt_bus: interrupt_bus.clone(),
            auditor: auditor.clone(),
        };

        KernelBuilder::new()
            .event_sink(events)
            .system_prompt(Arc::new(crate::kernel::prompt::agent_system_prompt))
            .service_handles(handles)
            .session_store(storage.clone())
            .main_model(main_service.clone())
            .auditor(auditor)
            .cache(cache.clone())
            .interrupt_bus(interrupt_bus)
            .extension(Arc::new(app_rpc))
            .register_kernel_plugins(crate::kernel::plugin::builtin_kernel_plugins())
            .register_plugins(crate::plugin::builtin_plugins())
            .build()
            .await
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

    /// 发起一轮 agent 回合（send_user_message 与「编辑用户消息后重发」共用）：
    /// 登记 turn 句柄、spawn loop、落盘、事件与审计收尾。
    async fn start_turn(
        &self,
        key: SessionKey,
        messages: Vec<Message>,
        forced_tool: Option<String>,
    ) -> Result<(), RpcError> {
        let signal = AbortSignal::new();
        let tools = self.registry.model_tools();
        // 会话上下文边界：从最近的「上一会话梗概」起算，旧会话内容不进模型上下文。
        let messages = scope_session_context(&messages);
        // 注入当前会话 ID：分叉会话 = 摘要节点（会话边界）的消息 UUID；根会话 = 链首消息 UUID。
        // 模型据此确认是否真的切换到了新会话（分叉后 ID 变化，会话内保持不变）。
        let session_id = messages
            .first()
            .map(|m| m.id.to_string())
            .unwrap_or_else(|| key.to_string());
        let mut scoped_messages = Vec::with_capacity(messages.len() + 1);
        scoped_messages.push(Message::system(format!("当前会话 ID：{session_id}")));
        scoped_messages.extend(messages);
        let messages = scoped_messages;
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
                forced_tool,
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
                    let next_active = compaction.as_ref().map(|c| c.tail_end).or(persisted_last);
                    if let Some(next) = next_active
                        && let Err(e) = store.set_active_path(&persist_key, Some(next)).await
                    {
                        events.emit(Event::Error {
                            message: format!("活跃路径推进失败：{e}"),
                        });
                    }
                    if let Err(e) = scheduler.on_turn_end(&persist_key, &outcome.messages).await {
                        events.emit(Event::Error {
                            message: format!("回合收尾失败：{e}"),
                        });
                    }
                    // 消息已落盘、活跃路径已推进：此刻通知前端刷新，链式渲染不会丢新消息。
                    events.emit(Event::TurnEnd {
                        stop_reason: outcome.stop_reason.clone(),
                    });
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
        Ok(())
    }

    /// 处理一个请求；返回需要写回 GUI 的响应帧（事件经 EventSink 另发）。
    pub async fn handle(&self, request: RpcRequest) -> Result<Option<RpcFrame>, RpcError> {
        match request.method {
            WireMethod::Generic(method) => self.handle_generic(request.id, method).await,
            WireMethod::Custom(custom) => {
                let params = custom_params(&custom);
                for ext in &self.extensions {
                    if let Some(result) = ext.handle(&custom.method, params.clone()).await? {
                        return Ok(Some(RpcFrame::Response {
                            id: request.id,
                            result: Some(result),
                            error: None,
                        }));
                    }
                }
                Err(RpcError::new(
                    "unknown_method",
                    format!("未知方法：{}", custom.method),
                ))
            }
        }
    }

    async fn handle_generic(
        &self,
        id: u64,
        method: Method,
    ) -> Result<Option<RpcFrame>, RpcError> {
        match method {
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
                let key = ctx.session_key;
                self.start_turn(key, ctx.messages, forced_wire).await?;

                Ok(Some(RpcFrame::Response {
                    id,
                    result: Some(json!({"accepted": true})),
                    error: None,
                }))
            }
            Method::TriggerCommand { entry, params } => {
                let result = self.dispatch.call_command(&entry, params).await;
                let frame = match result {
                    Ok(v) => RpcFrame::Response {
                        id,
                        result: Some(v),
                        error: None,
                    },
                    Err(e) => RpcFrame::Response {
                        id,
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
                    id,
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
                    id,
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
                    id,
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
                    id,
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
                    id,
                    result: Some(json!({ "tools": tools })),
                    error: None,
                }))
            }
            Method::EditMessage { message_id, text } => {
                let key = self.active_session_key().await?;
                // 仅 user 消息可编辑（storage 校验）：编辑 = 改完重发，
                // 保存后自动开启新一轮回答。
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
                self.start_turn(key, path.clone(), None).await?;
                Ok(Some(RpcFrame::Response {
                    id,
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
                    id,
                    result: Some(json!({
                        "session_key": key,
                        "messages": serde_json::to_value(&path).unwrap_or_default(),
                    })),
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
    use crate::kernel::audit::{Auditor, MemoryAuditSink};
    use crate::kernel::plugin::services::{
        AbortSignal, ModelChunk, ModelError, ModelResponse, ModelStream,
    };
    use crate::kernel::plugin::storage::MemoryStorage;

    #[test]
    fn rpc_wire_parses_generic_and_custom_methods() {
        let generic: RpcRequest =
            serde_json::from_str(r#"{"id":1,"method":"get_state"}"#).unwrap();
        assert!(matches!(generic.method, WireMethod::Generic(Method::GetState)));

        let custom: RpcRequest =
            serde_json::from_str(r#"{"id":2,"method":"check_balance"}"#).unwrap();
        let WireMethod::Custom(custom) = custom.method else {
            panic!("未知方法应落入 custom 兜底");
        };
        assert_eq!(custom.method, "check_balance");

        let compute: RpcRequest = serde_json::from_str(
            r#"{"id":3,"method":"compute_result","compute_id":9,"stdout":"ok","stderr":"","duration_ms":1}"#,
        )
        .unwrap();
        let WireMethod::Custom(compute) = compute.method else {
            panic!("compute_result 应落入 custom 兜底");
        };
        assert_eq!(compute.extra["compute_id"], 9);
        let merged = custom_params(&compute);
        assert_eq!(merged["compute_id"], 9);
        assert_eq!(merged["stdout"], "ok");
    }

    struct StubBuilderModel;

    #[async_trait]
    impl ModelService for StubBuilderModel {
        async fn stream(
            &self,
            _request: &ModelRequest,
            _signal: &AbortSignal,
        ) -> Result<ModelStream, ModelError> {
            Ok(Box::new(futures_util::stream::empty::<
                Result<ModelChunk, ModelError>,
            >()))
        }

        async fn complete(
            &self,
            _request: &ModelRequest,
            _signal: &AbortSignal,
        ) -> Result<ModelResponse, ModelError> {
            Err(ModelError::Transport("stub".into()))
        }
    }

    struct PingExtension;

    #[async_trait]
    impl RpcExtension for PingExtension {
        async fn handle(
            &self,
            method: &str,
            _params: Value,
        ) -> Result<Option<Value>, RpcError> {
            if method == "ping" {
                Ok(Some(json!({"pong": true})))
            } else {
                Ok(None)
            }
        }
    }

    #[tokio::test]
    async fn kernel_builder_assembles_and_routes_custom_method() {
        let store: Arc<dyn SessionStore> = Arc::new(MemoryStorage::new());
        let auditor = Auditor::new(Arc::new(MemoryAuditSink::default()));
        let kernel = KernelBuilder::new()
            .session_store(store)
            .main_model(Arc::new(StubBuilderModel))
            .auditor(auditor)
            .extension(Arc::new(PingExtension))
            .build()
            .await
            .unwrap();
        let frame = kernel
            .handle(RpcRequest::custom(1, "ping", json!({})))
            .await
            .unwrap()
            .expect("应有响应帧");
        assert!(serde_json::to_string(&frame).unwrap().contains("\"pong\":true"));
    }

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
