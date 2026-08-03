//! RPC（ADR-0013 / Q15）：stdio JSONL，JSON-RPC 2.0 风格（id + notification）。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::kernel::audit::{AuditRecord, Auditor};
use crate::kernel::compute::BridgeCompute;
use crate::kernel::dispatch::Dispatch;
use crate::kernel::events::{Event, EventSink};
use crate::kernel::logger::{Logger, LoggerHandle};
use crate::kernel::loop_mod::{AgentLoop, TurnInput, TurnOutcome};
use crate::kernel::memory::InMemoryMemory;
use crate::kernel::message::MessageId;
use crate::kernel::model::{RoutingModelService, build_main_service, build_vision_service};
use crate::kernel::registry::Registry;
use crate::kernel::services::{
    AbortSignal, ComputeHandle, MemoryHandle, ModelHandle, ServiceHandles, SessionStore,
    StorageHandle,
};
use crate::kernel::session::{
    InterruptBus, SessionKey, SessionScheduler, StubGuard, StubSummarizer, SystemClock,
};
use crate::kernel::settings::Settings;
use crate::kernel::storage::{AnyStorage, FileStorage};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    SendUserMessage {
        text: String,
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
        id: u64,
        stdout: String,
        stderr: String,
        duration_ms: u64,
    },
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
    state: Arc<Mutex<KernelState>>,
}

impl Kernel {
    /// 组装内核（M1 全内存 + 真模型适配器）。
    pub async fn new(events: Arc<dyn EventSink>) -> Result<Arc<Self>, String> {
        let settings = Settings::load()?;
        let data_root = Settings::data_root();
        Logger::init(settings.log_level, &data_root.join("logs"))?;
        let logger: LoggerHandle = Arc::new(Logger);

        let storage = Arc::new(match FileStorage::open(&data_root) {
            Ok(file) => AnyStorage::File(file),
            Err(e) => {
                eprintln!("[kernel] 文件存储打开失败，回退内存存储：{e}");
                AnyStorage::Mem(crate::kernel::storage::MemoryStorage::new())
            }
        });
        let memory = Arc::new(InMemoryMemory::new());
        let compute = Arc::new(BridgeCompute::new(events.clone()));
        let main_service = build_main_service(&settings);
        let vision_service = build_vision_service(&settings);

        let auditor = Auditor::new(storage.clone());
        let router = Arc::new(RoutingModelService::new(
            main_service.clone(),
            vision_service.clone(),
        ));
        let handles = ServiceHandles::default()
            .with_storage(StorageHandle::new(storage.clone()))
            .with_memory(MemoryHandle::new(memory.clone()))
            .with_compute(ComputeHandle::new(compute.clone()))
            .with_model(ModelHandle::new(
                router,
                std::time::Duration::from_secs(180),
                auditor.clone(),
            ));

        let registry = Arc::new(Registry::new(handles, logger));
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
        let loop_engine = Arc::new(AgentLoop::new(
            main_service,
            dispatch.clone(),
            auditor.clone(),
            events.clone(),
        ));

        let scheduler = Arc::new(SessionScheduler::new(
            storage.clone(),
            Arc::new(StubGuard::new()),
            Arc::new(SystemClock),
            Arc::new(StubSummarizer),
            InterruptBus::new(),
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
            state: Arc::new(Mutex::new(KernelState { turn: None })),
        }))
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn dispatch(&self) -> Arc<Dispatch> {
        self.dispatch.clone()
    }

    /// 当前是否有回合在跑（sidecar 收尾时轮询用）。
    pub async fn is_idle(&self) -> bool {
        self.state.lock().await.turn.is_none()
    }

    /// 处理一个请求；返回需要写回 GUI 的响应帧（事件经 EventSink 另发）。
    pub async fn handle(&self, request: RpcRequest) -> Result<Option<RpcFrame>, RpcError> {
        match request.method {
            Method::SendUserMessage { text } => {
                let mut state = self.state.lock().await;
                if state.turn.is_some() {
                    return Err(RpcError::new(
                        "turn_in_progress",
                        "当前有回合在跑，请先停止再发送新消息",
                    ));
                }
                let ctx = self
                    .scheduler
                    .on_new_message(&text)
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
                let state_for_task = self.state.clone();
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
                    };
                    let outcome: Result<TurnOutcome, _> = loop_engine.run_turn(input).await;
                    match outcome {
                        Ok(outcome) => {
                            for msg in &outcome.messages {
                                if let Err(e) = store.append_message(&key, msg).await {
                                    events.emit(Event::Error {
                                        message: format!("消息落盘失败：{e}"),
                                    });
                                }
                            }
                            if let Err(e) = scheduler.on_turn_end(&key, &outcome.messages).await {
                                events.emit(Event::Error {
                                    message: format!("回合收尾失败：{e}"),
                                });
                            }
                            auditor.record(AuditRecord::Lifecycle {
                                phase: "turn_finished".into(),
                            });
                        }
                        Err(e) => {
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
            Method::EditMessage { .. } | Method::SwitchBranch { .. } => Err(RpcError::new(
                "not_implemented",
                "消息树编辑/切分支为 M5 能力",
            )),
            Method::GetSettings => Err(RpcError::new(
                "not_implemented",
                "settings 读取为 M2/M5 能力",
            )),
            Method::SetSettings { .. } => Err(RpcError::new(
                "not_implemented",
                "settings 写入为 M2/M5 能力",
            )),
            Method::ComputeResult {
                id,
                stdout,
                stderr,
                duration_ms,
            } => {
                let delivered = self.compute.deliver(
                    id,
                    crate::kernel::services::ComputeResult {
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
