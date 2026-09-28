//! 错题领域模型（app 侧业务类型，不随 `so-lite-agent` 通用内核分发）。
//!
//! M1 解耦：`MistakeStore` 与错题数据结构从 `kernel::plugin::services`
//! 移到本模块；kernel 契约层经 `pub use` 兼容重导出，存储实现与业务插件
//! 以本模块为唯一事实源。`so-lite-agent` 已按 ADR-0037 迁出至独立仓库，
//! 本模块仍留在 mistake-agent（app 侧业务类型）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kernel::plugin::services::StorageError;

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
    /// 卡片标题：判分归档时由模型一并生成的一句话概括（≤16 字）。
    /// 存量错题没有这个字段（`None`），前端回退显示「学科 · 知识点」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    /// `None` = 不改；`Some("")`/纯空白 = 清空标题（回到前端回退展示）。
    pub title: Option<String>,
    pub question: Option<String>,
    pub student_answer: Option<String>,
    pub reference_answer: Option<Option<String>>,
    pub analysis: Option<String>,
    pub is_correct: Option<bool>,
    pub pinned: Option<bool>,
}

/// 标题归一化：trim 后为空一律视作「没有标题」（前端回退显示「学科 · 知识点」）。
/// 归档与编辑两条写入路径共用，免得空串在库里存出两种形态。
pub fn normalize_title(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
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
