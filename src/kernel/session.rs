//! Session scheduler（M1.5）：SessionKey、生命周期、守卫模型、交接摘要、空闲超时。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::kernel::message::{Message, MessageId};
use crate::kernel::prompt::{summarize_prompt, turn_decider_prompt};
use crate::kernel::services::{
    AbortSignal, ModelError, ModelKind, ModelRequest, ModelResponse, ModelService, ResponseFormat,
    SessionStore, StorageError,
};

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

/// 回合结束决策器：由主模型在回合结束时判断 continue / update_goal / start_new。
/// 守卫模型退役（ADR-0030）：切换决策全部归主模型——回合内经 session::switch 工具主动发起，
/// 回合结束经本决策器判断；模型错误 / 输出无法解析 / 超时一律降级 Continue（存疑即继续）。
pub struct LlmTurnDecider {
    model: Arc<dyn ModelService>,
    timeout: Duration,
    retries: usize,
    retry_delay: Duration,
    max_input_chars: usize,
}

impl LlmTurnDecider {
    pub fn new(model: Arc<dyn ModelService>) -> Self {
        Self {
            model,
            timeout: Duration::from_secs(60),
            retries: 2,
            retry_delay: Duration::from_secs(2),
            max_input_chars: 12000,
        }
    }

    /// 自定义重试参数（测试/调优用）。
    pub fn with_retry(mut self, retries: usize, delay: Duration) -> Self {
        self.retries = retries;
        self.retry_delay = delay;
        self
    }
}

/// 带重试的模型 complete：对瞬时错误（503/限流/超时）退避重试；
/// 系统性错误（鉴权/余额/模型下架）与取消不重试。
async fn complete_with_retry(
    model: &Arc<dyn ModelService>,
    request: &ModelRequest,
    timeout: Duration,
    retries: usize,
    delay: Duration,
) -> Result<ModelResponse, ModelError> {
    let mut attempt = 0usize;
    loop {
        match tokio::time::timeout(timeout, model.complete(request, &AbortSignal::new())).await {
            Ok(Ok(resp)) => return Ok(resp),
            Ok(Err(e)) => {
                if e.is_systemic() || matches!(e, ModelError::Cancelled) {
                    return Err(e);
                }
                if attempt >= retries {
                    return Err(e);
                }
                attempt += 1;
                log::warn!("模型调用失败（{attempt}/{retries} 重试）：{e}");
                tokio::time::sleep(delay * attempt as u32).await;
            }
            Err(_) => {
                if attempt >= retries {
                    return Err(ModelError::Timeout);
                }
                attempt += 1;
                log::warn!("模型调用超时（{attempt}/{retries} 重试）");
                tokio::time::sleep(delay * attempt as u32).await;
            }
        }
    }
}

fn guard_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["continue", "update_goal", "start_new"]},
            "goal": {"type": "string"}
        },
        "required": ["action", "goal"],
        "additionalProperties": false
    })
}

/// 解析守卫输出：容忍 ```json 围栏与首尾空白；解析失败返回 None（上层降级 Continue）。
fn parse_guard_decision(text: &str) -> Option<GuardDecision> {
    let text = text.trim();
    let stripped = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(text);
    let v: Value = serde_json::from_str(stripped).ok()?;
    let action = v["action"].as_str()?;
    let goal = || Goal {
        text: v["goal"].as_str().unwrap_or_default().trim().to_string(),
    };
    match action {
        "continue" => Some(GuardDecision::Continue),
        "update_goal" => Some(GuardDecision::UpdateGoal(goal())),
        "start_new" => Some(GuardDecision::StartNew(goal())),
        _ => None,
    }
}

#[async_trait]
impl GuardModel for LlmTurnDecider {
    async fn decide(&self, input: &GuardInput) -> Result<GuardDecision, GuardError> {
        let mut transcript = input.summary.clone();
        if transcript.len() > self.max_input_chars {
            transcript = transcript.chars().take(self.max_input_chars).collect();
        }
        let payload = json!({
            "goal": input.goal.as_ref().map(|g| &g.text),
            "transcript": transcript,
        });
        let request = ModelRequest {
            model: ModelKind::Main,
            messages: vec![
                Message::system(turn_decider_prompt()),
                Message::user(serde_json::to_string(&payload).unwrap_or_default()),
            ],
            tools: None,
            reasoning_effort: Some("none".into()),
            tool_choice: None,
            response_format: Some(ResponseFormat::JsonSchema {
                name: "guard_decision".to_string(),
                schema: guard_schema(),
            }),
        };
        let text = match complete_with_retry(
            &self.model,
            &request,
            self.timeout,
            self.retries,
            self.retry_delay,
        )
        .await
        {
            Ok(resp) => resp.text,
            Err(e) => return Err(GuardError::Model(e.to_string())),
        };
        parse_guard_decision(&text)
            .ok_or_else(|| GuardError::Parse(text.chars().take(200).collect()))
    }
}

fn message_text(msg: &Message) -> String {
    use crate::kernel::message::MessageKind;
    match &msg.kind {
        MessageKind::User { text, .. } => format!("用户：{text}"),
        MessageKind::Assistant { text } => format!("助手：{text}"),
        MessageKind::System { text } => format!("系统：{text}"),
        MessageKind::Reasoning { text, .. } => format!("推理：{text}"),
        MessageKind::ToolCall {
            entry,
            params,
            result,
            ..
        } => format!(
            "工具：{entry} 参数 {params} 结果 {:?}",
            result.as_ref().map(|v| v.to_string())
        ),
    }
}

/// 生产摘要器：LLM 生成 ≤300 字任务摘要；模型失败降级为 stub 式摘要。
pub struct LlmSummarizer {
    model: Arc<dyn ModelService>,
    timeout: Duration,
    max_input_chars: usize,
    /// 消息数少于该值时直接走 stub 摘要，不调 LLM（短会话无需生成式摘要）。
    min_messages_for_llm: usize,
    retries: usize,
    retry_delay: Duration,
}

impl LlmSummarizer {
    pub fn new(model: Arc<dyn ModelService>) -> Self {
        Self {
            model,
            timeout: Duration::from_secs(60),
            max_input_chars: 12000,
            min_messages_for_llm: 8,
            retries: 2,
            retry_delay: Duration::from_secs(2),
        }
    }

    pub fn with_retry(mut self, retries: usize, delay: Duration) -> Self {
        self.retries = retries;
        self.retry_delay = delay;
        self
    }

    fn fallback(messages: &[Message], goal: Option<&Goal>) -> String {
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

#[async_trait]
impl Summarizer for LlmSummarizer {
    async fn summarize(&self, messages: &[Message], goal: Option<&Goal>) -> String {
        if messages.len() < self.min_messages_for_llm {
            return Self::fallback(messages, goal);
        }
        let goal_text = goal
            .map(|g| g.text.clone())
            .unwrap_or_else(|| "（未记录目标）".into());
        let mut transcript = String::new();
        for msg in messages {
            let line = message_text(msg);
            if transcript.len() + line.len() > self.max_input_chars {
                transcript.push_str("…（已截断）");
                break;
            }
            transcript.push_str(&line);
            transcript.push('\n');
        }
        let request = ModelRequest {
            model: ModelKind::Main,
            messages: vec![
                Message::system(summarize_prompt()),
                Message::user(format!("目标：{goal_text}\n\n对话：\n{transcript}")),
            ],
            tools: None,
            reasoning_effort: Some("none".into()),
            tool_choice: None,
            response_format: None,
        };
        match complete_with_retry(
            &self.model,
            &request,
            self.timeout,
            self.retries,
            self.retry_delay,
        )
        .await
        {
            Ok(resp) => {
                let summary = resp.text.trim();
                if summary.is_empty() {
                    Self::fallback(messages, goal)
                } else {
                    summary.chars().take(300).collect()
                }
            }
            Err(e) => {
                log::warn!("摘要模型重试后仍失败，降级 stub 摘要：{e}");
                Self::fallback(messages, goal)
            }
        }
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
    decider: Arc<dyn GuardModel>,
    clock: Arc<dyn Clock>,
    summarizer: Arc<dyn Summarizer>,
    bus: InterruptBus,
    idle_timeout: Duration,
    max_switches_per_hour: usize,
    switch_times: Mutex<VecDeque<DateTime<Utc>>>,
    /// 切换时复制到新会话的旧会话最近消息数（无感知切换：历史上下文连续）。
    history_carryover: usize,
}

impl SessionScheduler {
    pub fn new(
        store: Arc<dyn SessionStore>,
        decider: Arc<dyn GuardModel>,
        clock: Arc<dyn Clock>,
        summarizer: Arc<dyn Summarizer>,
        bus: InterruptBus,
    ) -> Self {
        Self {
            store,
            decider,
            clock,
            summarizer,
            bus,
            idle_timeout: Duration::from_secs(12 * 60 * 60),
            max_switches_per_hour: 5,
            switch_times: Mutex::new(VecDeque::new()),
            history_carryover: 20,
        }
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    pub fn interrupt_bus(&self) -> InterruptBus {
        self.bus.clone()
    }

    /// 新消息到达：空闲超时检查 → 返回回合上下文。
    /// 切换决策不再在此发生（ADR-0030）：新消息默认继续当前会话；
    /// 切换只由「回合结束决策器」或主模型调用的 session::switch 工具发起。
    pub async fn on_new_message(&self, text: &str) -> Result<TurnContext, SchedulerError> {
        let now = self.clock.now();
        let metas = self.store.list_sessions().await?;
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .cloned();
        let Some(meta) = active else {
            // 首条消息：建新会话（不产生切换中断）。
            let (_, to) = self
                .switch_session(
                    None,
                    Goal {
                        text: text.chars().take(40).collect(),
                    },
                    now,
                )
                .await?;
            return self.append_user(&to, text).await;
        };

        let idle = now - meta.last_activity_at
            > chrono::Duration::from_std(self.idle_timeout).unwrap_or(chrono::Duration::hours(12));
        if idle {
            // 系统级空闲超时：开新会话（不依赖模型决策）。
            let goal = Goal {
                text: text.chars().take(40).collect(),
            };
            let (from, to) = self.switch_session(Some(&meta), goal.clone(), now).await?;
            self.record_switch(now);
            self.bus.send(Interrupt::SessionSwitched { from, to, goal });
            return self.append_user(&to, text).await;
        }

        // 默认继续当前会话（切换决策归主模型）。
        self.continue_in(&meta, text, now).await
    }

    /// 在活动会话中追加用户消息并推进 active_path。
    async fn continue_in(
        &self,
        meta: &SessionMeta,
        text: &str,
        now: DateTime<Utc>,
    ) -> Result<TurnContext, SchedulerError> {
        self.store.set_last_activity(&meta.key, now).await?;
        let mut user_msg = Message::user(text);
        let path = self.store.read_path(&meta.key).await?;
        user_msg.parent_id = path.last().map(|m| m.id);
        self.store.append_message(&meta.key, &user_msg).await?;
        self.store
            .set_active_path(&meta.key, Some(user_msg.id))
            .await?;
        Ok(TurnContext {
            session_key: meta.key,
            messages: self.store.read_path(&meta.key).await?,
        })
    }

    /// 在指定会话中追加用户消息并推进 active_path（新建/切换后使用）。
    async fn append_user(
        &self,
        key: &SessionKey,
        text: &str,
    ) -> Result<TurnContext, SchedulerError> {
        let mut user_msg = Message::user(text);
        let path = self.store.read_path(key).await?;
        user_msg.parent_id = path.last().map(|m| m.id);
        self.store.append_message(key, &user_msg).await?;
        self.store.set_active_path(key, Some(user_msg.id)).await?;
        Ok(TurnContext {
            session_key: *key,
            messages: self.store.read_path(key).await?,
        })
    }

    /// 回合结束：主模型决策 continue / update_goal / start_new（ADR-0030）。
    /// start_new 时立即执行切换（归档 + 历史携带）；决策失败默认 continue。
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
        // 决策输入：最近对话文本（不额外生成摘要）。
        let transcript = path
            .iter()
            .rev()
            .take(30)
            .rev()
            .map(message_text)
            .collect::<Vec<_>>()
            .join("\n");
        let input = GuardInput {
            goal: meta.goal.clone(),
            summary: transcript,
            new_text: None,
        };
        let decision = match self.decider.decide(&input).await {
            Ok(d) => {
                log::info!("回合结束决策：{d:?}");
                d
            }
            Err(e) => {
                log::warn!("回合结束决策失败，按 continue 处理：{e}");
                GuardDecision::Continue
            }
        };
        match decision {
            GuardDecision::UpdateGoal(goal) => {
                self.store.set_goal(key, &goal).await?;
                self.bus.send(Interrupt::GoalUpdated { goal });
            }
            GuardDecision::StartNew(goal) => {
                if goal.text.trim().is_empty() {
                    log::warn!("回合结束决策返回空目标，忽略 start_new");
                    return Ok(());
                }
                let now = self.clock.now();
                if !self.switch_allowed(now) {
                    log::warn!("切换过于频繁，回合结束 start_new 降级忽略");
                    return Ok(());
                }
                let (from, to) = self.switch_session(Some(&meta), goal.clone(), now).await?;
                self.record_switch(now);
                self.bus.send(Interrupt::SessionSwitched { from, to, goal });
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
        match old {
            Some(meta) => {
                // 摘要只生成一次：同时写入旧会话交接摘要与新会话梗概（历史路由保留完整记录）。
                let all = self.store.read_all(&meta.key).await?;
                let active = self.store.read_path(&meta.key).await?;
                let summary = self.summarizer.summarize(&all, meta.goal.as_ref()).await;
                let mut handoff = Message::system(format!("交接摘要：{summary}"));
                handoff.parent_id = all.last().map(|m| m.id);
                self.store.append_message(&meta.key, &handoff).await?;
                self.store.archive(&meta.key).await?;
                let to = SessionKey::new();
                self.create_session(to, goal.clone(), now).await?;
                // 无感知切换：新会话注入「梗概 + 旧会话最近消息副本」，
                // 模型上下文与消息树都保留连续历史（跨上上个会话由链式复制累积）。
                let handoff = Message::system(format!("上一会话梗概：{summary}"));
                self.store.append_message(&to, &handoff).await?;
                let copied = carry_history(&active, self.history_carryover);
                for (i, mut m) in copied.into_iter().enumerate() {
                    // 副本链挂在梗概之后，保证活跃路径（梗概 → 副本 → 新消息）完整。
                    if i == 0 {
                        m.parent_id = Some(handoff.id);
                    }
                    self.store.append_message(&to, &m).await?;
                }
                Ok((meta.key, to))
            }
            None => {
                let dummy = SessionKey::new();
                let new_key = SessionKey::new();
                self.create_session(new_key, goal.clone(), now).await?;
                Ok((dummy, new_key))
            }
        }
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

    /// 1 小时内是否还有切换额度（只检查，不记录）。
    fn switch_allowed(&self, now: DateTime<Utc>) -> bool {
        let mut times = self.switch_times.lock().expect("switch lock poisoned");
        while times
            .front()
            .is_some_and(|t| now - *t > chrono::Duration::hours(1))
        {
            times.pop_front();
        }
        times.len() < self.max_switches_per_hour
    }

    /// 记录一次成功切换。
    fn record_switch(&self, now: DateTime<Utc>) {
        self.switch_times
            .lock()
            .expect("switch lock poisoned")
            .push_back(now);
    }
}

/// 回合内主动切换会话（主模型 session::switch 工具调用；ADR-0030）。
#[async_trait]
pub trait SessionSwitch: Send + Sync {
    async fn switch(&self, goal: &str) -> Result<SessionKey, String>;
}

#[async_trait]
impl SessionSwitch for SessionScheduler {
    async fn switch(&self, goal: &str) -> Result<SessionKey, String> {
        let goal = goal.trim();
        if goal.is_empty() {
            return Err("新会话目标不能为空".into());
        }
        let now = self.clock.now();
        let metas = self
            .store
            .list_sessions()
            .await
            .map_err(|e| e.to_string())?;
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .cloned()
            .ok_or_else(|| "当前没有活动会话".to_string())?;
        if !self.switch_allowed(now) {
            return Err(format!(
                "切换过于频繁（1 小时内 >{} 次）",
                self.max_switches_per_hour
            ));
        }
        let goal = Goal {
            text: goal.to_string(),
        };
        let (from, to) = self
            .switch_session(Some(&active), goal.clone(), now)
            .await
            .map_err(|e| e.to_string())?;
        self.record_switch(now);
        self.bus.send(Interrupt::SessionSwitched { from, to, goal });
        Ok(to)
    }
}

/// 取活跃路径最近 N 条并重建副本 parent 链（副本自成一条链，挂到新会话树）。
fn carry_history(path: &[Message], keep: usize) -> Vec<Message> {
    let tail: Vec<Message> = path
        .iter()
        .rev()
        .take(keep)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut copied: Vec<Message> = Vec::with_capacity(tail.len());
    for mut m in tail {
        m.parent_id = copied.last().map(|c| c.id);
        copied.push(m);
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::services::{ModelChunk, ModelError, ModelResponse, ModelStream, TokenUsage};
    use crate::kernel::storage::MemoryStorage;
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
    async fn new_message_continues_current_session() {
        let (scheduler, _, store, _) = setup();
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let second = scheduler.on_new_message("生成周复习报告").await.unwrap();
        // ADR-0030：新消息默认继续当前会话；切换决策归主模型。
        assert_eq!(first.session_key, second.session_key);
        let metas = store.list_sessions().await.unwrap();
        assert_eq!(metas.len(), 1, "不应自动切换新会话");
        assert_eq!(metas[0].status, SessionStatus::Active, "唯一会话保持活动");
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
        // 首条建会话；随后主模型主动切换 5 次（达到 1 小时上限）。
        let first = scheduler.on_new_message("帮我看看这道题").await.unwrap();
        let mut last_key = first.session_key;
        for _ in 0..5 {
            last_key = scheduler.switch("新目标").await.unwrap();
        }
        // 第 6 次超限：拒绝并返回错误（调用方/模型可感知），不归档任何会话。
        assert!(scheduler.switch("再切一次").await.is_err());
        let metas = store.list_sessions().await.unwrap();
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .unwrap();
        assert_eq!(active.key, last_key);
        assert_eq!(metas.len(), 6, "1 个初始会话 + 5 次切换");
    }

    #[tokio::test]
    async fn switch_carries_recent_history_into_new_session() {
        let (scheduler, _, store, _) = setup();
        scheduler.on_new_message("帮我看看这道题").await.unwrap();
        scheduler.on_new_message("继续讲第二题").await.unwrap();
        let new_key = scheduler.switch("批改英语作业").await.unwrap();
        let msgs = store.read_path(&new_key).await.unwrap();
        assert!(
            matches!(
                msgs[0].kind,
                crate::kernel::message::MessageKind::System { .. }
            ),
            "新会话应以梗概开头"
        );
        assert!(
            matches!(
                msgs.last().unwrap().kind,
                crate::kernel::message::MessageKind::User { .. }
            ),
            "新会话以梗概开头"
        );
        let user_count = msgs
            .iter()
            .filter(|m| matches!(m.kind, crate::kernel::message::MessageKind::User { .. }))
            .count();
        assert!(
            user_count >= 2,
            "新会话应携带旧消息副本（2 条）：实际 {user_count}"
        );
        // parent 链连续：从末尾回溯能一路走到根（副本首条 parent=None）。
        let by_id: std::collections::HashMap<_, _> = msgs.iter().map(|m| (m.id, m)).collect();
        let mut cur = msgs.last().unwrap().parent_id;
        let mut steps = 0;
        while let Some(id) = cur {
            let m = by_id.get(&id).expect("parent 必须存在");
            cur = m.parent_id;
            steps += 1;
        }
        assert!(steps >= 2, "parent 链应连续：{steps}");
    }
}
