//! RPC（ADR-0013 / Q15）：GUI ↔ kernel 命令通道与事件流。

pub(crate) mod handlers;
mod protocol;

pub(crate) use handlers::{KernelState, TurnHandle, persist_turn_messages};
pub use protocol::{ForcedToolRequest, Method, RpcError, RpcFrame, RpcRequest};

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use crate::kernel::agent::cache::CacheTracker;
use crate::kernel::agent::dispatch::Dispatch;
use crate::kernel::agent::loop_mod::{AgentLoop, TurnInput, TurnOutcome};
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
use crate::kernel::registry::Registry;
use crate::kernel::settings::Settings;

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
        let memory: Arc<dyn MemoryService> = match FileMemoryService::open_default(storage.clone())
        {
            Ok(file_memory) => {
                // 旧存储布局迁移（ADR-0042）：中文路径 → base64url 段编码；失败不阻塞启动。
                if let Err(e) = file_memory.migrate_legacy_layout().await {
                    eprintln!("[kernel] 记忆布局迁移失败（继续启动）：{e}");
                }
                Arc::new(file_memory)
            }
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
            .with_storage(
                StorageHandle::new(storage.clone()).with_io(storage.clone(), storage.clone()),
            )
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
}
#[cfg(test)]
mod tests;
