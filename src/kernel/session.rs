//! Session scheduler（M1.5）：SessionKey、生命周期、守卫模型、交接摘要、空闲超时。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kernel::message::{Message, MessageId};
use crate::kernel::services::{SessionStore, StorageError};

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

// ---------- 守卫模型（Q17） ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardDecision {
    Continue,
    UpdateGoal(Goal),
    StartNew(Goal),
}

#[derive(Debug, Clone)]
pub struct GuardInput {
    pub goal: Option<Goal>,
    pub summary: String,
    pub new_text: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("守卫模型调用失败：{0}")]
    Model(String),
    #[error("守卫输出无法解析：{0}")]
    Parse(String),
}

#[async_trait]
pub trait GuardModel: Send + Sync {
    async fn decide(&self, input: &GuardInput) -> Result<GuardDecision, GuardError>;
}

/// M1.5 确定性 stub 守卫：关键词命中开新会话，否则继续。
/// 生产实现 = 独立小模型调用（M2/M3），接口不变。
pub struct StubGuard {
    start_new_keywords: Vec<String>,
}

impl StubGuard {
    pub fn new() -> Self {
        Self {
            start_new_keywords: vec!["报告".into(), "周报".into(), "新会话".into()],
        }
    }
}

impl Default for StubGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GuardModel for StubGuard {
    async fn decide(&self, input: &GuardInput) -> Result<GuardDecision, GuardError> {
        if let Some(text) = &input.new_text
            && self
                .start_new_keywords
                .iter()
                .any(|k| text.contains(k.as_str()))
        {
            return Ok(GuardDecision::StartNew(Goal {
                text: text.chars().take(40).collect(),
            }));
        }
        Ok(GuardDecision::Continue)
    }
}

// ---------- 时钟（可测试性） ----------

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone)]
pub struct FakeClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }
    pub fn advance(&self, d: Duration) {
        *self.now.lock().expect("clock poisoned") += chrono::Duration::from_std(d).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock poisoned")
    }
}

// ---------- 交接摘要 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffSummary {
    pub text: String,
}

#[async_trait]
pub trait Summarizer: Send + Sync {
    async fn summarize(&self, messages: &[Message], goal: Option<&Goal>) -> String;
}

/// M1.5 stub 摘要：真实实现 = 模型生成（M2）。
pub struct StubSummarizer;

#[async_trait]
impl Summarizer for StubSummarizer {
    async fn summarize(&self, messages: &[Message], goal: Option<&Goal>) -> String {
        let goal_text = goal
            .map(|g| g.text.clone())
            .unwrap_or_else(|| "（未记录目标）".into());
        format!(
            "上一个会话共 {} 条消息，会话目标：{}。",
            messages.len(),
            goal_text
        )
    }
}

// ---------- 环境变更中断（Q17d：内部中断 / InterruptBus） ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "interrupt", rename_all = "snake_case")]
pub enum Interrupt {
    SessionSwitched {
        from: SessionKey,
        to: SessionKey,
        goal: Goal,
    },
    GoalUpdated {
        goal: Goal,
    },
    SettingsChanged,
    MemoryChanged {
        path: String,
    },
    CompactionDone {
        session: SessionKey,
    },
}

#[derive(Clone)]
pub struct InterruptBus {
    queue: Arc<Mutex<std::collections::VecDeque<Interrupt>>>,
}

impl InterruptBus {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        }
    }

    pub fn send(&self, interrupt: Interrupt) {
        self.queue
            .lock()
            .expect("interrupt bus poisoned")
            .push_back(interrupt);
    }

    /// 消费全部待处理中断（agent loop 在回合边界调用）。
    pub fn take_all(&self) -> Vec<Interrupt> {
        std::mem::take(&mut *self.queue.lock().expect("interrupt bus poisoned"))
            .into_iter()
            .collect()
    }
}

impl Default for InterruptBus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- SessionScheduler ----------

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("storage 错误：{0}")]
    Storage(#[from] StorageError),
    #[error("守卫模型错误：{0}")]
    Guard(#[from] GuardError),
    #[error("内部错误：{0}")]
    Internal(String),
}

/// 一个回合的上下文：在哪个会话跑、活跃路径是什么。
#[derive(Debug, Clone)]
pub struct TurnContext {
    pub session_key: SessionKey,
    pub messages: Vec<Message>,
}

pub struct SessionScheduler {
    store: Arc<dyn SessionStore>,
    guard: Arc<dyn GuardModel>,
    clock: Arc<dyn Clock>,
    summarizer: Arc<dyn Summarizer>,
    bus: InterruptBus,
    idle_timeout: Duration,
    max_switches_per_hour: usize,
    switch_times: Mutex<VecDeque<DateTime<Utc>>>,
}

impl SessionScheduler {
    pub fn new(
        store: Arc<dyn SessionStore>,
        guard: Arc<dyn GuardModel>,
        clock: Arc<dyn Clock>,
        summarizer: Arc<dyn Summarizer>,
        bus: InterruptBus,
    ) -> Self {
        Self {
            store,
            guard,
            clock,
            summarizer,
            bus,
            idle_timeout: Duration::from_secs(12 * 60 * 60),
            max_switches_per_hour: 5,
            switch_times: Mutex::new(VecDeque::new()),
        }
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    pub fn interrupt_bus(&self) -> InterruptBus {
        self.bus.clone()
    }

    /// 新消息到达：空闲超时检查 → 守卫判断 → 返回回合上下文。
    pub async fn on_new_message(&self, text: &str) -> Result<TurnContext, SchedulerError> {
        let now = self.clock.now();
        let metas = self.store.list_sessions().await?;
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .cloned();

        let decision = match &active {
            None => Some(GuardDecision::StartNew(Goal {
                text: text.chars().take(40).collect(),
            })),
            Some(meta) => {
                let idle = now - meta.last_activity_at
                    > chrono::Duration::from_std(self.idle_timeout)
                        .unwrap_or(chrono::Duration::hours(12));
                if idle {
                    Some(GuardDecision::StartNew(Goal {
                        text: text.chars().take(40).collect(),
                    }))
                } else {
                    let summary = self
                        .summarizer
                        .summarize(&self.store.read_path(&meta.key).await?, meta.goal.as_ref())
                        .await;
                    let input = GuardInput {
                        goal: meta.goal.clone(),
                        summary,
                        new_text: Some(text.to_string()),
                    };
                    Some(self.guard.decide(&input).await?)
                }
            }
        };

        match decision {
            Some(GuardDecision::Continue) => {
                let meta = active.expect("continue 需要活动会话");
                self.store.set_last_activity(&meta.key, now).await?;
                let user_msg = Message::user(text);
                self.store.append_message(&meta.key, &user_msg).await?;
                Ok(TurnContext {
                    session_key: meta.key,
                    messages: self.store.read_path(&meta.key).await?,
                })
            }
            Some(GuardDecision::UpdateGoal(goal)) => {
                let meta = active.expect("update_goal 需要活动会话");
                self.store.set_goal(&meta.key, &goal).await?;
                self.store.set_last_activity(&meta.key, now).await?;
                let user_msg = Message::user(text);
                self.store.append_message(&meta.key, &user_msg).await?;
                self.bus.send(Interrupt::GoalUpdated { goal });
                Ok(TurnContext {
                    session_key: meta.key,
                    messages: self.store.read_path(&meta.key).await?,
                })
            }
            Some(GuardDecision::StartNew(goal)) => {
                let (from, to) = self
                    .switch_session(active.as_ref(), goal.clone(), now)
                    .await?;
                self.bus.send(Interrupt::SessionSwitched { from, to, goal });
                let user_msg = Message::user(text);
                self.store.append_message(&to, &user_msg).await?;
                Ok(TurnContext {
                    session_key: to,
                    messages: self.store.read_path(&to).await?,
                })
            }
            None => Err(SchedulerError::Internal("守卫没有返回决定".into())),
        }
    }

    /// 回合结束：只允许 Continue / UpdateGoal（Q17：StartNew 不在回合结束触发）。
    pub async fn on_turn_end(
        &self,
        key: &SessionKey,
        _outcome_messages: &[Message],
    ) -> Result<(), SchedulerError> {
        let meta = self
            .store
            .get_session(key)
            .await?
            .ok_or_else(|| SchedulerError::Internal("会话不存在".into()))?;
        let path = self.store.read_path(key).await?;
        let summary = self.summarizer.summarize(&path, meta.goal.as_ref()).await;
        let input = GuardInput {
            goal: meta.goal.clone(),
            summary,
            new_text: None,
        };
        match self.guard.decide(&input).await? {
            GuardDecision::UpdateGoal(goal) => {
                self.store.set_goal(key, &goal).await?;
                self.bus.send(Interrupt::GoalUpdated { goal });
            }
            GuardDecision::StartNew(_) => {
                log::warn!("守卫在回合结束返回 StartNew，按设计忽略（Q17）");
            }
            GuardDecision::Continue => {}
        }
        Ok(())
    }

    async fn switch_session(
        &self,
        old: Option<&SessionMeta>,
        goal: Goal,
        now: DateTime<Utc>,
    ) -> Result<(SessionKey, SessionKey), SchedulerError> {
        let from = match old {
            Some(meta) => {
                // 交接摘要写入旧会话，并注入新会话（历史路由保留完整记录）。
                let path = self.store.read_path(&meta.key).await?;
                let summary = self.summarizer.summarize(&path, meta.goal.as_ref()).await;
                let mut handoff = Message::system(format!("交接摘要：{summary}"));
                handoff.parent_id = path.last().map(|m| m.id);
                self.store.append_message(&meta.key, &handoff).await?;
                self.store.archive(&meta.key).await?;
                meta.key
            }
            None => {
                let dummy = SessionKey::new();
                // 无旧会话：不产生切换中断事件，直接建新会话。
                let new_key = SessionKey::new();
                self.create_session(new_key, goal.clone(), now).await?;
                return Ok((dummy, new_key));
            }
        };

        self.limit_switch_frequency(now)?;
        let to = SessionKey::new();
        self.create_session(to, goal.clone(), now).await?;
        // 新会话注入旧会话梗概。
        let path = self.store.read_all(&from).await?;
        let summary = self.summarizer.summarize(&path, None).await;
        let handoff = Message::system(format!("上一会话梗概：{summary}"));
        self.store.append_message(&to, &handoff).await?;
        Ok((from, to))
    }

    async fn create_session(
        &self,
        key: SessionKey,
        goal: Goal,
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        let mut meta = SessionMeta::new(key);
        meta.goal = Some(goal);
        meta.created_at = now;
        meta.last_activity_at = now;
        self.store.create_session(&key, &meta).await?;
        Ok(())
    }

    fn limit_switch_frequency(&self, now: DateTime<Utc>) -> Result<(), SchedulerError> {
        let mut times = self.switch_times.lock().expect("switch lock poisoned");
        while times
            .front()
            .is_some_and(|t| now - *t > chrono::Duration::hours(1))
        {
            times.pop_front();
        }
        if times.len() >= self.max_switches_per_hour {
            return Err(SchedulerError::Internal(
                "切换过于频繁，安全策略拒绝（存疑即继续）".into(),
            ));
        }
        times.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::storage::MemoryStorage;

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
    async fn first_message_creates_session() {
        let (scheduler, _, store, _) = setup();
        let ctx = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        assert_eq!(ctx.messages.len(), 1);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].status, SessionStatus::Active);
    }

    #[tokio::test]
    async fn idle_timeout_starts_new_session() {
        let (scheduler, clock, store, _) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        clock.advance(Duration::from_secs(13 * 60 * 60));
        let second = scheduler.on_new_message("生成周复习报告").await.unwrap();
        assert_ne!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 2);
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .unwrap();
        assert_eq!(active.key, second.session_key);
        // 新会话注入旧会话梗概（System 消息）。
        let msgs = store.read_path(&second.session_key).await.unwrap();
        assert!(matches!(
            msgs[0].kind,
            crate::kernel::message::MessageKind::System { .. }
        ));
    }

    #[tokio::test]
    async fn guard_keyword_starts_new_session() {
        let (scheduler, _, store, _) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let second = scheduler.on_new_message("生成周复习报告").await.unwrap();
        assert_ne!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        let first_meta = metas.iter().find(|m| m.key == first.session_key).unwrap();
        assert_eq!(first_meta.status, SessionStatus::Archived);
    }

    #[tokio::test]
    async fn interrupt_bus_receives_switch() {
        let (scheduler, _, _, bus) = setup();
        scheduler.on_new_message("帮我看看这道题").await.unwrap();
        scheduler.on_new_message("生成周复习报告").await.unwrap();
        let interrupts = bus.take_all();
        assert!(
            interrupts
                .iter()
                .any(|i| matches!(i, Interrupt::SessionSwitched { .. }))
        );
    }
}
