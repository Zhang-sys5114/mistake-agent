//! SessionScheduler：会话生命周期调度（M1.5 核心）。

use super::*;

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
        }
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    pub fn interrupt_bus(&self) -> InterruptBus {
        self.bus.clone()
    }

    /// 新消息到达（ADR-0032）：先由主模型判断要不要切换上下文，再返回回合上下文回答。
    /// 顺序：空闲超时（系统级）→ 主模型决策 start_new / update_goal / continue
    /// → 追加用户消息并进入回合；决策失败默认 continue（存疑即继续）。
    pub async fn on_new_message(&self, text: &str) -> Result<TurnContext, SchedulerError> {
        self.on_new_message_with_display(text, None).await
    }

    /// 带前端展示文本的新消息（force_tool 场景）：落盘的 user 消息同时携带
    /// display_text（渲染用）与 text（模型指令），二者分离（ADR-0007 修订）。
    pub async fn on_new_message_with_display(
        &self,
        text: &str,
        display_text: Option<&str>,
    ) -> Result<TurnContext, SchedulerError> {
        let now = self.clock.now();
        let metas = self.store.list_sessions().await?;
        let active = metas
            .iter()
            .find(|m| m.status == SessionStatus::Active)
            .cloned();
        let Some(meta) = active else {
            // 首条消息：建新会话（不产生切换中断）。
            let to = self
                .create_first_session(
                    Goal {
                        text: text.chars().take(40).collect(),
                    },
                    now,
                )
                .await?;
            return self.append_user(&to, text, display_text).await;
        };

        let idle = now - meta.last_activity_at
            > chrono::Duration::from_std(self.idle_timeout).unwrap_or(chrono::Duration::hours(12));
        if idle {
            // 系统级空闲超时：树内分叉新会话（不依赖模型决策）。
            let goal = Goal {
                text: text.chars().take(40).collect(),
            };
            return self
                .fork_branch(&meta, goal, Some(text), display_text, now)
                .await;
        }

        // 先判断（主模型）：这条新消息要不要切换上下文。
        let path = self.store.read_path(&meta.key).await?;
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
            new_text: Some(text.to_string()),
        };
        let decision = match self.decider.decide(&input).await {
            Ok(d) => {
                log::info!("新消息上下文决策：{d:?}");
                d
            }
            Err(e) => {
                log::warn!("新消息上下文决策失败，按 continue 兜底：{e}");
                GuardDecision::Continue
            }
        };
        match decision {
            GuardDecision::StartNew(goal) => {
                if goal.text.trim().is_empty() {
                    log::warn!("新消息决策返回空目标，降级继续当前会话");
                    return self.continue_in(&meta, text, display_text, now).await;
                }
                if !self.switch_allowed(now) {
                    log::warn!("切换过于频繁，新消息 start_new 降级忽略");
                    return self.continue_in(&meta, text, display_text, now).await;
                }
                // 树内分叉新会话：当前叶子下挂「摘要节点 + 新用户消息」。
                self.fork_branch(&meta, goal, Some(text), display_text, now)
                    .await
            }
            GuardDecision::UpdateGoal(goal) => {
                if !goal.text.trim().is_empty() {
                    self.store.set_goal(&meta.key, &goal).await?;
                    self.bus.send(Interrupt::GoalUpdated { goal });
                }
                self.continue_in(&meta, text, display_text, now).await
            }
            GuardDecision::Continue => self.continue_in(&meta, text, display_text, now).await,
        }
    }

    /// 在活动会话中追加用户消息并推进 active_path。
    async fn continue_in(
        &self,
        meta: &SessionMeta,
        text: &str,
        display_text: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<TurnContext, SchedulerError> {
        self.store.set_last_activity(&meta.key, now).await?;
        let mut user_msg = Message::user_with_display(text, display_text.map(str::to_string));
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
        display_text: Option<&str>,
    ) -> Result<TurnContext, SchedulerError> {
        let mut user_msg = Message::user_with_display(text, display_text.map(str::to_string));
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
                // 回合结束分叉：只挂摘要节点，下一条新消息从该子树继续。
                self.fork_branch(&meta, goal, None, None, now).await?;
            }
            GuardDecision::Continue => {}
        }
        Ok(())
    }

    /// 创建根会话（仅首条消息调用）：根会话没有摘要节点，直接以用户消息开头。
    async fn create_first_session(
        &self,
        goal: Goal,
        now: DateTime<Utc>,
    ) -> Result<SessionKey, SchedulerError> {
        let new_key = SessionKey::new();
        self.create_session(new_key, goal, now).await?;
        Ok(new_key)
    }

    /// 树内分叉新会话：在当前叶子节点下挂一棵「会话子树」——
    /// 先追加摘要节点（上一会话梗概，同时是模型上下文边界），再挂新用户消息（如有）。
    /// 摘要节点与当前叶子互为兄弟版本（旧分支保留，< / > 可切回）；不新建 SessionKey。
    async fn fork_branch(
        &self,
        meta: &SessionMeta,
        goal: Goal,
        text: Option<&str>,
        display_text: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<TurnContext, SchedulerError> {
        let all = self.store.read_all(&meta.key).await?;
        let path = self.store.read_path(&meta.key).await?;
        let summary = self.summarizer.summarize(&all, meta.goal.as_ref()).await;
        let leaf = path.last();
        // 摘要节点挂成当前叶子的子节点：新会话在模型回复之后线性延续，
        // 旧回复保留在链上（不产生「回复=1、新会话=2」的版本分裂）。
        let summary_parent = leaf.map(|m| m.id);
        let mut summary_msg =
            Message::system_with_display(format!("上一会话梗概：{summary}"), None);
        summary_msg.parent_id = summary_parent;
        summary_msg.created_at = now;
        self.store.append_message(&meta.key, &summary_msg).await?;
        self.store
            .set_active_path(&meta.key, Some(summary_msg.id))
            .await?;
        if let Some(text) = text {
            let mut user_msg = Message::user_with_display(text, display_text.map(str::to_string));
            user_msg.parent_id = Some(summary_msg.id);
            user_msg.created_at = now;
            self.store.append_message(&meta.key, &user_msg).await?;
            self.store
                .set_active_path(&meta.key, Some(user_msg.id))
                .await?;
        }
        self.store.set_goal(&meta.key, &goal).await?;
        self.record_switch(now);
        self.bus.send(Interrupt::SessionSwitched {
            from: meta.key,
            to: meta.key,
            goal,
        });
        Ok(TurnContext {
            session_key: meta.key,
            messages: self.store.read_path(&meta.key).await?,
        })
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
        let ctx = self
            .fork_branch(&active, goal.clone(), None, None, now)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ctx.session_key)
    }
}

/// 判断是否为「会话摘要」节点（新会话子树的根，模型上下文边界）。
pub fn is_session_summary(m: &Message) -> bool {
    matches!(
        &m.kind,
        MessageKind::System { text, .. } if text.starts_with("上一会话梗概：")
    )
}

/// 会话上下文边界：从最近的「上一会话梗概」节点起算（含该节点）。
/// 摘要之前的祖先（旧会话内容）不进模型上下文——新会话只带本会话内容。
pub fn scope_session_context(messages: &[Message]) -> Vec<Message> {
    if let Some(idx) = messages.iter().rposition(is_session_summary) {
        messages[idx..].to_vec()
    } else {
        messages.to_vec()
    }
}
