//! 内核服务契约与受控句柄（ADR-0001/0014/0016；Q5/Q6/Q8/Q9/Q10 定稿）。
//!
//! 本文件是**内核插件公共契约聚合**（对应 Linux 内核的公共头/注册表入口）：
//! - `ServiceId` 内核插件标识（Storage/Memory/Compute/Model）；
//! - `ServiceHandles` 类型化封闭容器，只装四个服务；
//! - 注入给插件的 `*Handle` 是受控视图（如 StorageHandle 只有错题本）。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::kernel::audit::{AuditRecord, Auditor};
use crate::kernel::events::{Event, EventSink};
use crate::kernel::message::{Message, MessageId};

/// 服务标识：v2 封闭集合（ADR-0014）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceId {
    Storage,
    Memory,
    Compute,
    Model,
}

// ---------- 取消信号（SIGTERM 通道；SIGKILL 由 dispatch 任务 abort 承担） ----------

#[derive(Clone)]
pub struct AbortSignal {
    token: CancellationToken,
}

impl AbortSignal {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    pub fn from_token(token: CancellationToken) -> Self {
        Self { token }
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn cancelled(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- Model 契约（Q6 + ADR-0014/0020） ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Main,
    Vision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonObject,
    /// DeepSeek Responses API `text.format` 的 json_schema 模式（服务端强制结构）。
    JsonSchema {
        name: String,
        schema: serde_json::Value,
    },
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub model: ModelKind,
    pub messages: Vec<Message>,
    pub tools: Option<Vec<ToolSchema>>,
    /// DeepSeek 思考模式 effort（none/minimal/low/medium/high/xhigh/max）。
    pub reasoning_effort: Option<String>,
    pub response_format: Option<ResponseFormat>,
    /// 工具选择策略：强制调用指定工具时用 Function{name}（API 要求关闭思考模式）。
    pub tool_choice: Option<ToolChoice>,
}

/// 工具选择策略（OpenAI Responses 兼容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    Required,
    /// 强制调用指定工具（wire name）。
    Function {
        name: String,
    },
}

impl ModelRequest {
    pub fn chat(model: ModelKind, messages: Vec<Message>) -> Self {
        Self {
            model,
            messages,
            tools: None,
            reasoning_effort: None,
            response_format: None,
            tool_choice: None,
        }
    }
}

/// 模型可见工具（wire name + JSON Schema）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Message,
    FunctionCall,
    Reasoning,
}

#[derive(Debug, Clone)]
pub enum ModelChunk {
    TextDelta(String),
    ReasoningDelta(String),
    /// 推理 item 开始（携带 id，后续轮次必须按 id 回传给 API）。
    ReasoningItemStart {
        id: String,
    },
    ToolCallStart {
        index: usize,
        call_id: String,
        name: String,
    },
    ToolCallDelta {
        index: usize,
        data: String,
    },
    ItemDone {
        kind: ItemKind,
    },
    /// 完整响应中的 token 用量（response.completed 携带）。
    Usage(TokenUsage),
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSpec {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// 缓存命中的输入 tokens（Responses API: input_tokens_details.cached_tokens；
    /// Chat Completions: prompt_cache_hit_tokens）。
    pub cached_tokens: Option<u64>,
    /// 缓存未命中的输入 tokens（Responses API 由 input - cached 推算；
    /// Chat Completions: prompt_cache_miss_tokens）。
    pub cache_miss_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCallSpec>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    #[error("鉴权失败：{0}")]
    AuthFailed(String),
    #[error("余额或配额不足：{0}")]
    QuotaExceeded(String),
    #[error("模型不存在或已下架：{0}")]
    ModelNotFound(String),
    #[error("请求超时")]
    Timeout,
    #[error("被取消")]
    Cancelled,
    #[error("限流：{0}")]
    RateLimited(String),
    #[error("传输错误：{0}")]
    Transport(String),
    #[error("协议错误：{0}")]
    Protocol(String),
    #[error("配置缺失：{0}")]
    Config(String),
}

impl ModelError {
    /// 系统性错误：重试/换参数无意义，应中断回合（Q17c）。
    pub fn is_systemic(&self) -> bool {
        matches!(
            self,
            ModelError::AuthFailed(_) | ModelError::QuotaExceeded(_) | ModelError::ModelNotFound(_)
        )
    }
}

pub type ModelStream = Box<dyn Stream<Item = Result<ModelChunk, ModelError>> + Send + Unpin>;

/// 纯净 provider 抽象（不管超时/审计；护栏在包装层与 loop）。
#[async_trait]
pub trait ModelService: Send + Sync {
    async fn stream(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelStream, ModelError>;

    async fn complete(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelResponse, ModelError> {
        use futures_util::StreamExt;

        let mut stream = self.stream(request, signal).await?;
        let mut text = String::new();
        let mut calls: Vec<(usize, ToolCallSpec)> = Vec::new();
        let mut usage_holder: Option<TokenUsage> = None;
        while let Some(chunk) = stream.next().await {
            match chunk? {
                ModelChunk::TextDelta(d) => text.push_str(&d),
                ModelChunk::ToolCallStart {
                    index,
                    call_id,
                    name,
                } => {
                    calls.push((
                        index,
                        ToolCallSpec {
                            call_id,
                            name,
                            arguments: String::new(),
                        },
                    ));
                }
                ModelChunk::ToolCallDelta { index, data } => {
                    if let Some((_, spec)) = calls.iter_mut().find(|(i, _)| *i == index) {
                        spec.arguments.push_str(&data);
                    }
                }
                ModelChunk::Usage(usage) => {
                    usage_holder = Some(usage);
                }
                _ => {}
            }
        }
        Ok(ModelResponse {
            text,
            tool_calls: calls.into_iter().map(|(_, spec)| spec).collect(),
            usage: usage_holder,
        })
    }
}

/// 注入用户插件的模型受控句柄：只暴露带超时 + abort + 审计的 complete。
#[derive(Clone)]
pub struct ModelHandle {
    inner: Arc<dyn ModelService>,
    timeout: Duration,
    auditor: Auditor,
}

impl ModelHandle {
    pub fn new(inner: Arc<dyn ModelService>, timeout: Duration, auditor: Auditor) -> Self {
        Self {
            inner,
            timeout,
            auditor,
        }
    }

    pub async fn complete(
        &self,
        request: &ModelRequest,
        signal: &AbortSignal,
    ) -> Result<ModelResponse, ModelError> {
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(self.timeout, self.inner.complete(request, signal)).await;
        let duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(Ok(resp)) => {
                self.auditor.record(AuditRecord::LlmCall {
                    provider: "handle".into(),
                    model: match request.model {
                        ModelKind::Main => "main".into(),
                        ModelKind::Vision => "vision".into(),
                    },
                    kind: "complete".into(),
                    tokens_in: resp.usage.as_ref().and_then(|u| u.input_tokens),
                    tokens_out: resp.usage.as_ref().and_then(|u| u.output_tokens),
                    duration_ms,
                    ok: true,
                });
                Ok(resp)
            }
            Ok(Err(e)) => {
                self.auditor.record(AuditRecord::LlmCall {
                    provider: "handle".into(),
                    model: match request.model {
                        ModelKind::Main => "main".into(),
                        ModelKind::Vision => "vision".into(),
                    },
                    kind: "complete".into(),
                    tokens_in: None,
                    tokens_out: None,
                    duration_ms,
                    ok: false,
                });
                Err(e)
            }
            Err(_) => Err(ModelError::Timeout),
        }
    }
}

// ---------- Storage 契约（Q8：角色拆分，插件只见 MistakeStore） ----------

use crate::kernel::agent::session::{Goal, SessionKey, SessionMeta};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("会话不存在：{0}")]
    SessionNotFound(SessionKey),
    #[error("错题不存在：{0}")]
    MistakeNotFound(String),
    #[error("已存在：{0}")]
    AlreadyExists(String),
    #[error("数据损坏：{0}")]
    Corrupt(String),
    #[error("IO 错误：{0}")]
    Io(String),
    #[error("路径非法：{0}")]
    InvalidPath(String),
    #[error("内部错误：{0}")]
    Internal(String),
}

// ---------- 域内文件 IO（ADR-0042 磁盘 IO 铁律） ----------

/// 数据根目录下的域（storage 拥有的子目录）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Mistakes,
    Sessions,
    Memory,
    Data,
    Uploads,
}

impl Domain {
    pub fn as_dir(self) -> &'static str {
        match self {
            Domain::Mistakes => "mistakes",
            Domain::Sessions => "sessions",
            Domain::Memory => "memory",
            Domain::Data => "data",
            Domain::Uploads => "uploads",
        }
    }
}

/// 相对路径：构造即校验（ADR-0042）。
///
/// 白名单字符校验，不做任何路径语义解析——段必须以 `[a-zA-Z0-9]` 开头和结尾，
/// 中间只允许 `[a-zA-Z0-9._-]`；空段、`.`、`..`、尾点、首点全部拒绝。
/// 构造后类型上不可能表示目录遍历（fail-closed，无规范化/替换通道）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelPath {
    segments: Vec<String>,
}

impl RelPath {
    pub fn parse(raw: &str) -> Result<Self, StorageError> {
        if raw.is_empty() {
            return Err(StorageError::InvalidPath("路径为空".into()));
        }
        let mut segments = Vec::new();
        for seg in raw.split('/') {
            let ok = !seg.is_empty()
                && seg
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                && seg
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
            if !ok {
                return Err(StorageError::InvalidPath(format!(
                    "非法路径段：{seg}（仅允许字母数字开头结尾，中间 [a-zA-Z0-9._-]）"
                )));
            }
            segments.push(seg.to_string());
        }
        Ok(Self { segments })
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn as_str(&self) -> String {
        self.segments.join("/")
    }
}

/// 数据根目录域内文件能力（ADR-0042）：内核插件的唯一磁盘通道。
///
/// 实现（storage）内部负责：域根拼接 + canonicalize 兜底（防符号链接逃逸）+ 原子写 + 审计。
/// 用户插件永远不持有本 trait——它们只见 `StorageHandle` 的语义方法。
#[async_trait]
pub trait DomainIo: Send + Sync {
    async fn read(&self, domain: Domain, rel: &RelPath) -> Result<Vec<u8>, StorageError>;
    async fn write(&self, domain: Domain, rel: &RelPath, bytes: &[u8]) -> Result<(), StorageError>;
    async fn remove(&self, domain: Domain, rel: &RelPath) -> Result<(), StorageError>;
    /// 递归删除子树（memory 的 remove 语义）。
    async fn remove_tree(&self, domain: Domain, rel: &RelPath) -> Result<(), StorageError>;
    /// 列出域内全部条目（递归，返回 `/` 分隔的相对路径）。
    async fn list(&self, domain: Domain) -> Result<Vec<String>, StorageError>;

    /// 读取域内**历史**文件（路径可能含非 ASCII 段，过不了 RelPath 白名单）。
    /// 仅启动引导迁移调用（ADR-0042 存储布局迁移），新代码一律走 RelPath；
    /// 实现侧做宽松校验（拒绝 `..`/`\`/绝对路径/空段）+ canonicalize 兜底 + 审计。
    async fn read_legacy(&self, domain: Domain, legacy_rel: &str) -> Result<Vec<u8>, StorageError>;

    /// 删除域内**历史**文件（同 read_legacy 的约束，仅迁移用）。
    async fn remove_legacy(&self, domain: Domain, legacy_rel: &str) -> Result<(), StorageError>;
}

/// 系统 temp 暂存文件能力（ADR-0042）：附件暂存（`mistake-agent-` 前缀白名单）。
///
/// 与 DomainIo 解耦：硬编码 `std::env::temp_dir()`，只管 temp 里自己前缀的文件，
/// 不做目录管理，只做受限读写；读删都记审计。
#[async_trait]
pub trait TmpIo: Send + Sync {
    async fn read_staged(&self, path: &str) -> Result<Vec<u8>, StorageError>;
    async fn remove_staged(&self, path: &str) -> Result<(), StorageError>;
}

/// 会话持久化：只给 kernel 内部（Session scheduler / loop / 压缩）。
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create_session(
        &self,
        key: &SessionKey,
        meta: &SessionMeta,
    ) -> Result<(), StorageError>;
    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionMeta>, StorageError>;
    async fn append_message(&self, key: &SessionKey, msg: &Message) -> Result<(), StorageError>;
    async fn read_path(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError>;
    async fn read_all(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError>;
    /// 设置活跃路径末端（消息树分支切换；None = 退化为线性全链）。
    async fn set_active_path(
        &self,
        key: &SessionKey,
        message_id: Option<MessageId>,
    ) -> Result<(), StorageError>;
    /// 在 message_id 处派生新分支：消息复制新 id（parent 不变、文本替换），
    /// 编辑点之后的旧消息保留在 JSONL 但不再属于活跃路径（ADR-0007 历史不截断）。
    /// 返回新活跃路径。
    async fn derive_branch(
        &self,
        key: &SessionKey,
        message_id: MessageId,
        text: &str,
    ) -> Result<Vec<Message>, StorageError>;
    /// 切换到以 message_id 为末端的活跃路径（沿 parent 链回溯）。
    async fn switch_branch(
        &self,
        key: &SessionKey,
        message_id: MessageId,
    ) -> Result<Vec<Message>, StorageError>;
    /// 压缩接入：把摘要消息追加进会话，并把 tail_start（保留段首条）的 parent 改挂到摘要下，
    /// 使活跃路径变为 `摘要 → 保留段 → …`，旧前缀仍在 JSONL 但不进上下文。
    async fn splice_compaction(
        &self,
        key: &SessionKey,
        summary: &Message,
        tail_start: MessageId,
    ) -> Result<(), StorageError>;
    async fn set_goal(&self, key: &SessionKey, goal: &Goal) -> Result<(), StorageError>;
    async fn archive(&self, key: &SessionKey) -> Result<(), StorageError>;
    async fn list_sessions(&self) -> Result<Vec<SessionMeta>, StorageError>;
    async fn set_last_activity(
        &self,
        key: &SessionKey,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StorageError>;
}

// ---------- 错题模型 ----------

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MistakeId(pub Uuid);

impl std::fmt::Display for MistakeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mistake {
    pub id: MistakeId,
    pub subject: String,
    pub knowledge_point: String,
    pub question: String,
    pub student_answer: String,
    pub reference_answer: Option<String>,
    pub is_correct: bool,
    pub analysis: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MistakeFilter {
    pub subject: Option<String>,
    pub knowledge_point: Option<String>,
    pub is_correct: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MistakePatch {
    pub subject: Option<String>,
    pub knowledge_point: Option<String>,
    pub question: Option<String>,
    pub student_answer: Option<String>,
    pub reference_answer: Option<Option<String>>,
    pub analysis: Option<String>,
    pub is_correct: Option<bool>,
    pub pinned: Option<bool>,
}

/// 错题本：用户插件唯一可见的 storage 面。
#[async_trait]
pub trait MistakeStore: Send + Sync {
    async fn save(&self, mistake: &Mistake) -> Result<MistakeId, StorageError>;
    async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError>;
    async fn list(&self, filter: &MistakeFilter) -> Result<Vec<Mistake>, StorageError>;
    async fn update(&self, id: &MistakeId, patch: &MistakePatch) -> Result<(), StorageError>;
    async fn remove(&self, id: &MistakeId) -> Result<(), StorageError>;
    async fn remove_many(&self, ids: &[MistakeId]) -> Result<usize, StorageError> {
        let mut deleted = 0usize;
        for id in ids {
            match self.remove(id).await {
                Ok(()) => deleted += 1,
                Err(StorageError::MistakeNotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(deleted)
    }
}

/// Storage 服务组合接口：kernel 持有全量，插件拿 StorageHandle 视图。
pub trait StorageService: SessionStore + MistakeStore + crate::kernel::audit::AuditSink {}

/// 注入插件的 storage 受控句柄：只有错题本 + 语义化文件 IO（附件暂存 / 数据文件）。
#[derive(Clone)]
pub struct StorageHandle {
    inner: Arc<dyn MistakeStore>,
    tmp: Option<Arc<dyn TmpIo>>,
    domain: Option<Arc<dyn DomainIo>>,
}

impl StorageHandle {
    pub fn new(inner: Arc<dyn MistakeStore>) -> Self {
        Self {
            inner,
            tmp: None,
            domain: None,
        }
    }

    /// 注入 IO 能力（Kernel::new 装配时调用；测试/回退时可缺省）。
    pub fn with_io(mut self, tmp: Arc<dyn TmpIo>, domain: Arc<dyn DomainIo>) -> Self {
        self.tmp = Some(tmp);
        self.domain = Some(domain);
        self
    }

    /// 附件暂存读取（TmpIo 语义方法，白名单在实现内）。
    pub async fn read_staged(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let tmp = self
            .tmp
            .as_ref()
            .ok_or_else(|| StorageError::Internal("storage 未注入 TmpIo 能力".into()))?;
        tmp.read_staged(path).await
    }

    /// 附件暂存删除（TmpIo 语义方法）。
    pub async fn remove_staged(&self, path: &str) -> Result<(), StorageError> {
        let tmp = self
            .tmp
            .as_ref()
            .ok_or_else(|| StorageError::Internal("storage 未注入 TmpIo 能力".into()))?;
        tmp.remove_staged(path).await
    }

    /// 教学数据文件读取（DomainIo data 域）。
    pub async fn read_data_file(&self, name: &str) -> Result<String, StorageError> {
        let domain = self
            .domain
            .as_ref()
            .ok_or_else(|| StorageError::Internal("storage 未注入 DomainIo 能力".into()))?;
        let rel = RelPath::parse(name)?;
        let bytes = domain.read(Domain::Data, &rel).await?;
        String::from_utf8(bytes)
            .map_err(|e| StorageError::Corrupt(format!("数据文件非 UTF-8：{e}")))
    }

    /// 教学数据文件写入（DomainIo data 域，原子写）。
    pub async fn write_data_file(&self, name: &str, content: &str) -> Result<(), StorageError> {
        let domain = self
            .domain
            .as_ref()
            .ok_or_else(|| StorageError::Internal("storage 未注入 DomainIo 能力".into()))?;
        let rel = RelPath::parse(name)?;
        domain.write(Domain::Data, &rel, content.as_bytes()).await
    }

    pub async fn save(&self, m: &Mistake) -> Result<MistakeId, StorageError> {
        self.inner.save(m).await
    }
    pub async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError> {
        self.inner.get(id).await
    }
    pub async fn list(&self, f: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
        self.inner.list(f).await
    }
    pub async fn update(&self, id: &MistakeId, p: &MistakePatch) -> Result<(), StorageError> {
        self.inner.update(id, p).await
    }
    pub async fn remove(&self, id: &MistakeId) -> Result<(), StorageError> {
        self.inner.remove(id).await
    }
    pub async fn remove_many(&self, ids: &[MistakeId]) -> Result<usize, StorageError> {
        self.inner.remove_many(ids).await
    }
}

// ---------- Memory 契约（Q9） ----------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MemoryPath {
    segments: Vec<String>,
}

impl MemoryPath {
    pub fn parse(raw: &str) -> Result<Self, MemoryError> {
        let raw_segments: Vec<&str> = raw.split('/').collect();
        if raw_segments.is_empty() || raw_segments.iter().any(|s| s.trim().is_empty()) {
            return Err(MemoryError::InvalidPath(
                "路径不能为空、不能以 / 开头或结尾，也不能含空段".into(),
            ));
        }
        let segments: Vec<String> = raw_segments.iter().map(|s| s.trim().to_string()).collect();
        if segments
            .iter()
            .any(|s| s == "." || s == ".." || s.contains('\\') || s.chars().any(|c| c.is_control()))
        {
            return Err(MemoryError::InvalidPath(format!("非法路径段：{raw}")));
        }
        Ok(Self { segments })
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn as_str(&self) -> String {
        self.segments.join("/")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryView {
    Listing(Vec<String>),
    Entry { path: MemoryPath, content: String },
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("非法路径：{0}")]
    InvalidPath(String),
    #[error("条目不存在：{0}")]
    NotFound(String),
    #[error("IO 错误：{0}")]
    Io(String),
}

#[async_trait]
pub trait MemoryService: Send + Sync {
    async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError>;
    async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError>;
    /// 删除整棵子树（Q9 语义）。
    async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError>;
}

#[derive(Clone)]
pub struct MemoryHandle {
    inner: Arc<dyn MemoryService>,
    events: Arc<dyn EventSink>,
    auditor: Auditor,
}

impl MemoryHandle {
    /// 无观测构造（测试/插件单测用；事件与审计为 no-op，对外签名不变）。
    pub fn new(inner: Arc<dyn MemoryService>) -> Self {
        Self {
            inner,
            events: Arc::new(crate::kernel::events::MemoryEventSink::default()),
            auditor: Auditor::new(Arc::new(crate::kernel::audit::MemoryAuditSink::default())),
        }
    }

    /// 生产构造：save/remove 发 MemoryChanged 事件并审计，show 审计 MemoryRead。
    pub fn with_observability(
        inner: Arc<dyn MemoryService>,
        events: Arc<dyn EventSink>,
        auditor: Auditor,
    ) -> Self {
        Self {
            inner,
            events,
            auditor,
        }
    }
    pub async fn save(&self, p: &MemoryPath, c: &str) -> Result<(), MemoryError> {
        let result = self.inner.save(p, c).await;
        if result.is_ok() {
            let path = p.as_str();
            self.auditor
                .record(AuditRecord::MemoryWrite { path: path.clone() });
            self.events.emit(Event::MemoryChanged { path });
        }
        result
    }
    pub async fn show(&self, p: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
        self.auditor.record(AuditRecord::MemoryRead {
            path: p.map(|x| x.as_str()),
        });
        self.inner.show(p).await
    }
    pub async fn remove(&self, p: &MemoryPath) -> Result<(), MemoryError> {
        let result = self.inner.remove(p).await;
        if result.is_ok() {
            let path = p.as_str();
            self.auditor
                .record(AuditRecord::MemoryRemove { path: path.clone() });
            self.events.emit(Event::MemoryChanged { path });
        }
        result
    }
}

// ---------- Compute 契约（Q10） ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeRequest {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeResult {
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ComputeError {
    #[error("执行端不可用（GUI/Pyodide 未连接）")]
    BackendUnavailable,
    #[error("执行超时")]
    Timeout,
    #[error("超出资源限制：{0}")]
    ResourceLimit(String),
    #[error("传输错误：{0}")]
    Transport(String),
}

/// 代码执行失败（Python traceback）走结果，不走错误（Q10）。
#[async_trait]
pub trait ComputeService: Send + Sync {
    async fn run(
        &self,
        request: &ComputeRequest,
        signal: &AbortSignal,
    ) -> Result<ComputeResult, ComputeError>;
}

#[derive(Clone)]
pub struct ComputeHandle {
    inner: Arc<dyn ComputeService>,
}

impl ComputeHandle {
    pub fn new(inner: Arc<dyn ComputeService>) -> Self {
        Self { inner }
    }
    pub async fn run(
        &self,
        request: &ComputeRequest,
        signal: &AbortSignal,
    ) -> Result<ComputeResult, ComputeError> {
        self.inner.run(request, signal).await
    }
}

// ---------- ServiceHandles：类型化封闭容器（Q5 修订） ----------

#[derive(Default, Clone)]
pub struct ServiceHandles {
    storage: Option<StorageHandle>,
    memory: Option<MemoryHandle>,
    compute: Option<ComputeHandle>,
    model: Option<ModelHandle>,
}

impl ServiceHandles {
    pub fn storage(&self) -> Option<&StorageHandle> {
        self.storage.as_ref()
    }
    pub fn memory(&self) -> Option<&MemoryHandle> {
        self.memory.as_ref()
    }
    pub fn compute(&self) -> Option<&ComputeHandle> {
        self.compute.as_ref()
    }
    pub fn model(&self) -> Option<&ModelHandle> {
        self.model.as_ref()
    }

    pub fn with_storage(mut self, h: StorageHandle) -> Self {
        self.storage = Some(h);
        self
    }
    pub fn with_memory(mut self, h: MemoryHandle) -> Self {
        self.memory = Some(h);
        self
    }
    pub fn with_compute(mut self, h: ComputeHandle) -> Self {
        self.compute = Some(h);
        self
    }
    pub fn with_model(mut self, h: ModelHandle) -> Self {
        self.model = Some(h);
        self
    }

    pub fn available(&self) -> HashSet<ServiceId> {
        let mut set = HashSet::new();
        if self.storage.is_some() {
            set.insert(ServiceId::Storage);
        }
        if self.memory.is_some() {
            set.insert(ServiceId::Memory);
        }
        if self.compute.is_some() {
            set.insert(ServiceId::Compute);
        }
        if self.model.is_some() {
            set.insert(ServiceId::Model);
        }
        set
    }

    /// 按能力声明过滤：插件只拿到声明过的服务（结构上受限）。
    pub fn filter(&self, requires: &[ServiceId]) -> ServiceHandles {
        let mut out = ServiceHandles::default();
        for id in requires {
            match id {
                ServiceId::Storage => out.storage = self.storage.clone(),
                ServiceId::Memory => out.memory = self.memory.clone(),
                ServiceId::Compute => out.compute = self.compute.clone(),
                ServiceId::Model => out.model = self.model.clone(),
            }
        }
        out
    }
}
