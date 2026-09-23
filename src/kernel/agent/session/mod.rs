//! Session scheduler（M1.5）：SessionKey、生命周期、交接摘要、空闲超时。

// ---------- 会话类型（Key/Goal/Status/Meta） ----------

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kernel::message::{Message, MessageId};
use crate::kernel::plugin::services::{
    AbortSignal, ModelError, ModelRequest, ModelResponse, ModelService, SessionStore, StorageError,
};
use crate::kernel::prompt::summarize_prompt;

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
mod interrupt;
mod scheduler;
mod summarize;

pub use clock::{Clock, FakeClock, SystemClock};
pub use interrupt::{Interrupt, InterruptBus};
pub use scheduler::{CreatedSession, SchedulerError, SessionScheduler, TurnContext};
pub use summarize::{HandoffSummary, LlmSummarizer, StubSummarizer, Summarizer};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::events::{Event, MemoryEventSink};
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

    fn setup() -> (
        SessionScheduler,
        FakeClock,
        MemoryStorage,
        InterruptBus,
        Arc<MemoryEventSink>,
    ) {
        let store = MemoryStorage::new();
        let clock = FakeClock::new(Utc::now());
        let bus = InterruptBus::new();
        let events = Arc::new(MemoryEventSink::default());
        let scheduler = SessionScheduler::new(
            Arc::new(store.clone()),
            Arc::new(clock.clone()),
            Arc::new(StubSummarizer),
            bus.clone(),
            events.clone(),
        );
        (scheduler, clock, store, bus, events)
    }

    #[tokio::test]
    async fn display_text_persisted_on_forced_tool_message() {
        // force_tool 场景：text（模型指令）与 display_text（前端展示）分离落盘。
        let (scheduler, _, store, _, _) = setup();
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

    #[tokio::test]
    async fn first_message_creates_session() {
        let (scheduler, _, store, _, _) = setup();
        let ctx = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        assert_eq!(ctx.messages.len(), 1);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].status, SessionStatus::Active);
    }

    #[tokio::test]
    async fn idle_timeout_emits_event_and_continues() {
        let (scheduler, clock, store, _, events) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        events.take();
        clock.advance(Duration::from_secs(13 * 60 * 60));
        let second = scheduler.on_new_message("生成周复习报告").await.unwrap();
        // 空闲超时不再自动分叉：留在原会话，只发提示事件（ADR-0044）。
        assert_eq!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].status, SessionStatus::Active);
        let msgs = store.read_path(&second.session_key).await.unwrap();
        assert!(
            !msgs.iter().any(|m| matches!(
                m.kind,
                MessageKind::System { ref text, .. } if text.contains("上一会话梗概")
            )),
            "不应再插入摘要节点"
        );
        assert!(matches!(
            msgs.last().unwrap().kind,
            MessageKind::User { .. }
        ));
        let emitted = events.take();
        assert!(
            emitted
                .iter()
                .any(|e| matches!(e, Event::SessionIdle { .. })),
            "应发出空闲提示事件：{emitted:?}"
        );
    }

    #[tokio::test]
    async fn new_message_continues_current_session() {
        let (scheduler, _, store, _, _) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let second = scheduler.on_new_message("继续讲第二题").await.unwrap();
        // 没有任何自动切换：新消息一律继续当前会话。
        assert_eq!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1, "不应自动切换新会话");
        assert_eq!(metas[0].status, SessionStatus::Active, "唯一会话保持活动");
    }

    #[tokio::test]
    async fn create_new_session_archives_previous_and_returns_new_key() {
        let (scheduler, _, store, _, _) = setup();
        let old = scheduler.on_new_message("帮我看看这道题").await.unwrap();

        let created = scheduler.create_new_session(None, false).await.unwrap();

        assert_ne!(created.key, old.session_key, "新会话应为独立 SessionKey");
        assert_eq!(created.archived, Some(old.session_key));
        assert!(!created.summary_attached);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 2);
        let archived = metas.iter().find(|m| m.key == old.session_key).unwrap();
        assert_eq!(archived.status, SessionStatus::Archived);
        assert!(archived.archived_at.is_some());
        // 单 Active 不变量：查找活动会话的地方都取第一个匹配。
        assert_eq!(
            metas
                .iter()
                .filter(|m| m.status == SessionStatus::Active)
                .count(),
            1
        );
        assert_eq!(
            metas.iter().find(|m| m.key == created.key).unwrap().status,
            SessionStatus::Active
        );
    }

    #[tokio::test]
    async fn create_new_session_without_active_session() {
        let (scheduler, _, store, _, _) = setup();
        let created = scheduler
            .create_new_session(
                Some(Goal {
                    text: "线性代数".into(),
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(created.archived, None);
        assert!(!created.summary_attached);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].goal.as_ref().unwrap().text, "线性代数");
    }

    #[tokio::test]
    async fn create_new_session_carry_summary_attaches_summary_first() {
        let (scheduler, _, store, _, _) = setup();
        let old = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        scheduler.on_new_message("继续讲第二题").await.unwrap();

        let created = scheduler.create_new_session(None, true).await.unwrap();

        assert!(created.summary_attached);
        let msgs = store.read_path(&created.key).await.unwrap();
        assert_eq!(msgs.len(), 1, "新会话首条消息即为交接摘要");
        assert!(matches!(
            msgs[0].kind,
            MessageKind::System { ref text, .. } if text.contains("上一会话梗概")
        ));
        // 旧会话本身不因携带摘要而被写入。
        let old_msgs = store.read_path(&old.session_key).await.unwrap();
        assert!(!old_msgs.iter().any(|m| matches!(
            m.kind,
            MessageKind::System { ref text, .. } if text.contains("上一会话梗概")
        )));
    }

    #[tokio::test]
    async fn create_new_session_carry_summary_skips_empty_session() {
        let (scheduler, _, store, _, _) = setup();
        // 先建一个空会话（无任何消息），再携带摘要新建。
        let old = scheduler.create_new_session(None, false).await.unwrap();

        let created = scheduler.create_new_session(None, true).await.unwrap();

        assert!(!created.summary_attached, "空会话没有可交接的内容");
        assert_eq!(created.archived, Some(old.key));
        assert!(store.read_path(&created.key).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn on_new_message_appends_under_handoff_summary() {
        let (scheduler, _, store, _, events) = setup();
        scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let created = scheduler.create_new_session(None, true).await.unwrap();
        events.take();

        let ctx = scheduler.on_new_message("换一道新题").await.unwrap();

        assert_eq!(ctx.session_key, created.key, "新消息应落在新会话");
        let msgs = store.read_path(&created.key).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(
            matches!(
                msgs[0].kind,
                MessageKind::System { ref text, .. } if text.contains("上一会话梗概")
            ),
            "首条应为交接摘要"
        );
        assert_eq!(
            msgs[1].parent_id,
            Some(msgs[0].id),
            "用户消息应挂在摘要节点下"
        );
        assert!(events.take().is_empty(), "刚活动过，不应触发空闲提示");
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
}
