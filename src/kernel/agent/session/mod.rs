//! Session scheduler（M1.5）：SessionKey、生命周期、守卫模型、交接摘要、空闲超时。

// ---------- 会话类型（Key/Goal/Status/Meta） ----------

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::kernel::message::{Message, MessageId, MessageKind};
use crate::kernel::plugin::services::{
    AbortSignal, ModelError, ModelKind, ModelRequest, ModelResponse, ModelService, ResponseFormat,
    SessionStore, StorageError,
};
use crate::kernel::prompt::{summarize_prompt, turn_decider_prompt};

// ---------- SessionKey ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionKey(pub Uuid);

impl SessionKey {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionKey {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ---------- 会话元数据 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub key: SessionKey,
    pub goal: Option<Goal>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub last_activity_at: DateTime<Utc>,
    pub active_path: Option<MessageId>,
}

impl SessionMeta {
    pub fn new(key: SessionKey) -> Self {
        let now = Utc::now();
        Self {
            key,
            goal: None,
            status: SessionStatus::Active,
            created_at: now,
            archived_at: None,
            last_activity_at: now,
            active_path: None,
        }
    }
}

mod clock;
mod guard;
mod interrupt;
mod scheduler;
mod summarize;

pub use clock::{Clock, FakeClock, SystemClock};
pub use guard::{GuardDecision, GuardError, GuardInput, GuardModel, LlmTurnDecider, StubGuard};
pub use interrupt::{Interrupt, InterruptBus};
pub use scheduler::{SchedulerError, SessionScheduler, SessionSwitch, TurnContext};
pub use scheduler::{is_session_summary, scope_session_context};
pub use summarize::{HandoffSummary, LlmSummarizer, StubSummarizer, Summarizer};

// 子模块间共享的内部函数（同 crate 可见）。
pub(crate) use guard::complete_with_retry;
#[cfg(test)]
pub(crate) use guard::parse_guard_decision;
pub(crate) use summarize::message_text;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::message::MessageKind;
    use crate::kernel::plugin::services::{
        ModelChunk, ModelError, ModelResponse, ModelStream, TokenUsage,
    };
    use crate::kernel::plugin::storage::MemoryStorage;
    use std::collections::VecDeque;

    /// 脚本模型：按队列顺序返回文本或错误（测试守卫/摘要器用）。
    struct ScriptedModel {
        queue: std::sync::Mutex<VecDeque<Result<String, String>>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl ScriptedModel {
        fn new(responses: Vec<Result<String, String>>) -> Self {
            Self {
                queue: std::sync::Mutex::new(responses.into()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ModelService for ScriptedModel {
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
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let next = self
                .queue
                .lock()
                .expect("scripted model poisoned")
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()));
            match next {
                Ok(text) => Ok(ModelResponse {
                    text,
                    tool_calls: Vec::new(),
                    usage: Some(TokenUsage {
                        input_tokens: Some(1),
                        output_tokens: Some(1),
                        ..Default::default()
                    }),
                }),
                Err(e) => Err(ModelError::Transport(e)),
            }
        }
    }

    fn setup() -> (SessionScheduler, FakeClock, MemoryStorage, InterruptBus) {
        let store = MemoryStorage::new();
        let clock = FakeClock::new(Utc::now());
        let bus = InterruptBus::new();
        let scheduler = SessionScheduler::new(
            Arc::new(store.clone()),
            Arc::new(StubGuard::new()),
            Arc::new(clock.clone()),
            Arc::new(StubSummarizer),
            bus.clone(),
        );
        (scheduler, clock, store, bus)
    }

    #[tokio::test]
    async fn display_text_persisted_on_forced_tool_message() {
        // force_tool 场景：text（模型指令）与 display_text（前端展示）分离落盘。
        let (scheduler, _, store, _) = setup();
        let ctx = scheduler
            .on_new_message_with_display(
                "请调用工具 memory::show 处理：数学/向量组的线性相关性",
                Some("翻看记忆：数学/向量组的线性相关性"),
            )
            .await
            .unwrap();
        let msgs = store.read_path(&ctx.session_key).await.unwrap();
        let user = msgs
            .iter()
            .find_map(|m| match &m.kind {
                MessageKind::User {
                    text, display_text, ..
                } => Some((text.clone(), display_text.clone())),
                _ => None,
            })
            .expect("应有 user 消息");
        assert_eq!(
            user.0,
            "请调用工具 memory::show 处理：数学/向量组的线性相关性"
        );
        assert_eq!(user.1.as_deref(), Some("翻看记忆：数学/向量组的线性相关性"));
    }

    /// 确定性 continue 守卫：测试“新消息默认继续当前会话”时替代关键词版 StubGuard。
    struct ContinueGuard;

    #[async_trait]
    impl GuardModel for ContinueGuard {
        async fn decide(&self, _input: &GuardInput) -> Result<GuardDecision, GuardError> {
            Ok(GuardDecision::Continue)
        }
    }

    #[tokio::test]
    async fn first_message_creates_session() {
        let (scheduler, _, store, _) = setup();
        let ctx = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        assert_eq!(ctx.messages.len(), 1);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].status, SessionStatus::Active);
    }

    #[tokio::test]
    async fn idle_timeout_forks_session_branch() {
        let (scheduler, clock, store, _) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        clock.advance(Duration::from_secs(13 * 60 * 60));
        let second = scheduler.on_new_message("生成周复习报告").await.unwrap();
        // 空闲超时不再开新会话：同一棵树内分叉。
        assert_eq!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].status, SessionStatus::Active);
        // 活跃路径 = [..., 摘要节点, 新用户消息]
        let msgs = store.read_path(&second.session_key).await.unwrap();
        assert!(matches!(
            msgs[msgs.len() - 2].kind,
            crate::kernel::message::MessageKind::System { ref text, .. }
                if text.contains("上一会话梗概")
        ));
        assert!(matches!(
            msgs.last().unwrap().kind,
            crate::kernel::message::MessageKind::User { .. }
        ));
    }

    #[tokio::test]
    async fn new_message_continues_current_session() {
        let store = MemoryStorage::new();
        let clock = FakeClock::new(Utc::now());
        let bus = InterruptBus::new();
        let scheduler = SessionScheduler::new(
            Arc::new(store.clone()),
            Arc::new(ContinueGuard),
            Arc::new(clock.clone()),
            Arc::new(StubSummarizer),
            bus.clone(),
        );
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let second = scheduler.on_new_message("继续讲第二题").await.unwrap();
        // 主模型决策 continue：新消息继续当前会话（ADR-0032）。
        assert_eq!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1, "不应自动切换新会话");
        assert_eq!(metas[0].status, SessionStatus::Active, "唯一会话保持活动");
    }

    #[tokio::test]
    async fn new_message_start_new_forks_branch() {
        // StubGuard 命中“报告”关键词 → start_new：树内分叉（摘要节点 + 新用户消息）。
        let (scheduler, _, store, bus) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let second = scheduler.on_new_message("生成周复习报告").await.unwrap();
        assert_eq!(first.session_key, second.session_key, "分叉不新建会话");
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1, "仍是同一个会话");
        assert_eq!(metas[0].status, SessionStatus::Active);
        let msgs = store.read_path(&second.session_key).await.unwrap();
        assert!(
            msgs.iter().any(|m| {
                matches!(
                    m.kind,
                    crate::kernel::message::MessageKind::System { ref text, .. }
                        if text.contains("上一会话梗概")
                )
            }),
            "分叉点应有会话摘要节点"
        );
        assert!(
            matches!(
                msgs.last().unwrap().kind,
                crate::kernel::message::MessageKind::User { .. }
            ),
            "新用户消息应挂到摘要之后"
        );
        let interrupts = bus.take_all();
        assert!(
            interrupts
                .iter()
                .any(|i| matches!(i, Interrupt::SessionSwitched { .. })),
            "应发出会话切换中断"
        );
    }

    #[tokio::test]
    async fn interrupt_bus_receives_switch() {
        let (scheduler, _, _, bus) = setup();
        scheduler.on_new_message("帮我看看这道题").await.unwrap();
        scheduler.switch("批改英语作业").await.unwrap();
        let interrupts = bus.take_all();
        assert!(
            interrupts
                .iter()
                .any(|i| matches!(i, Interrupt::SessionSwitched { .. }))
        );
    }

    #[test]
    fn parse_guard_decision_accepts_json_and_fences() {
        assert!(matches!(
            parse_guard_decision(r#"{"action":"continue","goal":""}"#),
            Some(GuardDecision::Continue)
        ));
        let d = parse_guard_decision(
            "```json\n{\"action\":\"start_new\",\"goal\":\"批改英语作业\"}\n```",
        );
        assert!(matches!(d, Some(GuardDecision::StartNew(_))));
        assert!(parse_guard_decision("不是 JSON").is_none());
    }

    #[tokio::test]
    async fn llm_turn_decider_returns_start_new() {
        let model = Arc::new(ScriptedModel::new(vec![Ok(
            r#"{"action":"start_new","goal":"批改英语作业"}"#.into(),
        )]));
        let decider = LlmTurnDecider::new(model);
        let decision = decider
            .decide(&GuardInput {
                goal: Some(Goal {
                    text: "复习数学".into(),
                }),
                summary: "最近对话：做完三道绝对值题".into(),
                new_text: None,
            })
            .await
            .unwrap();
        match decision {
            GuardDecision::StartNew(goal) => assert_eq!(goal.text, "批改英语作业"),
            _ => panic!("应 start_new"),
        }
    }

    #[tokio::test]
    async fn turn_end_decision_failure_falls_back_to_continue() {
        let store = MemoryStorage::new();
        let clock = FakeClock::new(Utc::now());
        let bus = InterruptBus::new();
        let decider = Arc::new(LlmTurnDecider::new(Arc::new(ScriptedModel::new(vec![
            Err("模型 500".into()),
        ]))));
        let scheduler = SessionScheduler::new(
            Arc::new(store.clone()),
            decider,
            Arc::new(clock.clone()),
            Arc::new(StubSummarizer),
            bus.clone(),
        );
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        // 回合结束决策失败 → 存疑即继续：不切会话。
        scheduler
            .on_turn_end(&first.session_key, &[])
            .await
            .unwrap();
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].status, SessionStatus::Active);
        let leftovers = bus.take_all();
        assert!(leftovers.is_empty(), "意外中断：{leftovers:?}");
    }

    #[tokio::test]
    async fn llm_summarizer_falls_back_on_model_error() {
        // 8+ 条消息才走 LLM；连续失败 3 次（含重试）后降级 stub。
        let model = Arc::new(ScriptedModel::new(vec![
            Err("HTTP 503 Service Unavailable".into()),
            Err("HTTP 503 Service Unavailable".into()),
            Err("HTTP 503 Service Unavailable".into()),
        ]));
        let messages: Vec<Message> = (0..8).map(|i| Message::user(format!("消息 {i}"))).collect();
        let summarizer = LlmSummarizer::new(model).with_retry(2, Duration::ZERO);
        let text = summarizer.summarize(&messages, None).await;
        assert!(text.contains("共 8 条消息"));
    }

    #[tokio::test]
    async fn llm_turn_decider_retries_transient_errors() {
        // 第一次 503 → 重试成功，决策器返回 start_new。
        let model = Arc::new(ScriptedModel::new(vec![
            Err("HTTP 503 Service Unavailable".into()),
            Ok(r#"{"action":"start_new","goal":"批改英语作业"}"#.into()),
        ]));
        let decider = LlmTurnDecider::new(model).with_retry(2, Duration::ZERO);
        let decision = decider
            .decide(&GuardInput {
                goal: Some(Goal {
                    text: "复习数学".into(),
                }),
                summary: "最近对话：作业批改完成".into(),
                new_text: None,
            })
            .await
            .unwrap();
        assert!(matches!(decision, GuardDecision::StartNew(_)));
    }

    #[tokio::test]
    async fn llm_summarizer_retries_transient_errors() {
        let model = Arc::new(ScriptedModel::new(vec![
            Err("HTTP 503 Service Unavailable".into()),
            Ok("本会话完成三套英语作业批改，错题已归档。".into()),
        ]));
        let messages: Vec<Message> = (0..8).map(|i| Message::user(format!("消息 {i}"))).collect();
        let summarizer = LlmSummarizer::new(model).with_retry(2, Duration::ZERO);
        let text = summarizer.summarize(&messages, None).await;
        assert!(text.contains("三套英语作业批改"));
    }

    #[tokio::test]
    async fn short_session_summary_skips_llm() {
        let model = Arc::new(ScriptedModel::new(vec![]));
        let summarizer = LlmSummarizer::new(model.clone());
        let text = summarizer
            .summarize(&[Message::user("你好"), Message::user("继续")], None)
            .await;
        assert!(text.contains("共 2 条消息"));
        assert_eq!(model.call_count(), 0, "短会话摘要不应调用模型");
    }

    #[tokio::test]
    async fn frequency_limit_rejects_excess_switches() {
        let store = MemoryStorage::new();
        let clock = FakeClock::new(Utc::now());
        let bus = InterruptBus::new();
        let scheduler = SessionScheduler::new(
            Arc::new(store.clone()),
            Arc::new(StubGuard::new()),
            Arc::new(clock.clone()),
            Arc::new(StubSummarizer),
            bus.clone(),
        );
        // 首条建会话；随后分叉 5 次（达到 1 小时上限）。
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let mut last_key = first.session_key;
        for _ in 0..5 {
            last_key = scheduler.switch("新目标").await.unwrap();
        }
        // 第 6 次超限：拒绝并返回错误（调用方/模型可感知），不产生新分支。
        assert!(scheduler.switch("再切一次").await.is_err());
        let metas = store.list_sessions().await.unwrap();
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .unwrap();
        assert_eq!(active.key, last_key);
        assert_eq!(metas.len(), 1, "分叉不新建会话");
    }

    #[tokio::test]
    async fn switch_forks_branch_with_summary() {
        let (scheduler, _, store, _) = setup();
        scheduler.on_new_message("帮我看看这道题").await.unwrap();
        scheduler.on_new_message("继续讲第二题").await.unwrap();
        let key = scheduler.switch("批改英语作业").await.unwrap();
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1, "切换不新建会话");
        let msgs = store.read_path(&key).await.unwrap();
        assert!(
            matches!(
                msgs.last().unwrap().kind,
                crate::kernel::message::MessageKind::System { ref text, .. }
                    if text.contains("上一会话梗概")
            ),
            "切换后活跃路径末尾应为会话摘要节点"
        );
    }

    #[test]
    fn scope_session_context_cuts_at_summary() {
        let u1 = Message::user("u1");
        let a1 = Message::assistant("a1");
        let mut s = Message::system_with_display("上一会话梗概：摘要", None);
        s.parent_id = Some(a1.id);
        let u2 = Message::user("u2");
        let scoped = scope_session_context(&[u1.clone(), a1.clone(), s.clone(), u2.clone()]);
        assert_eq!(scoped.len(), 2, "从摘要节点起算");
        assert_eq!(scoped[0].id, s.id);
        assert_eq!(scoped[1].id, u2.id);
        // 无摘要节点（根会话）时原样返回。
        let full = scope_session_context(&[u1.clone(), a1.clone()]);
        assert_eq!(full.len(), 2);
    }
}
