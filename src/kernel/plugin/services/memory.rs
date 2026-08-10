//! Memory 契约（Q9）。

use super::*;

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
