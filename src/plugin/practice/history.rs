//! practice 插件：练习历史与防重复（P2 功能 8）。
//!
//! 练习记录（item/知识点/对错/时间）以 JSON 数组落 memory 的 `practice/history`
//! 单键（路由式浏览，本地单用户，无并发写）；出题前读取"近期已掌握"集合，
//! 模板 / 真题池 / LLM 生成均避开，减少重复无效刷题。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::kernel::plugin::services::{MemoryHandle, MemoryPath, MemoryView};

use super::templates::Difficulty;

/// memory 中的练习历史键。
pub const HISTORY_PATH: &str = "practice/history";

/// 防膨胀：仅保留最近 N 条记录。
const MAX_RECORDS: usize = 200;

/// 出题避开的"已掌握"窗口（天）。
pub const MASTERED_WINDOW_DAYS: u32 = 30;

/// 单条练习记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PracticeRecord {
    /// 题目标识：模板 template_id、exam:item_id 或 llm_freeform；无来源时用题目哈希。
    pub item_id: String,
    pub knowledge_point: String,
    pub difficulty: Difficulty,
    pub correct: bool,
    /// RFC3339 时间戳。
    pub created_at: String,
}

impl PracticeRecord {
    pub fn new(
        item_id: &str,
        knowledge_point: &str,
        difficulty: Difficulty,
        correct: bool,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            knowledge_point: knowledge_point.into(),
            difficulty,
            correct,
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

fn history_path() -> MemoryPath {
    MemoryPath::parse(HISTORY_PATH).expect("常量路径必然合法")
}

/// 读取全部练习历史（键不存在或内容损坏视为空，不阻断主流程）。
async fn load_history(memory: &MemoryHandle) -> Vec<PracticeRecord> {
    let path = history_path();
    match memory.show(Some(&path)).await {
        Ok(MemoryView::Entry { content, .. }) => {
            serde_json::from_str(&content).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// 追加一条练习记录并写回（保留最近 MAX_RECORDS 条）。
pub async fn record_attempt(
    memory: &MemoryHandle,
    item_id: &str,
    knowledge_point: &str,
    difficulty: Difficulty,
    correct: bool,
) {
    let path = history_path();
    let mut records = load_history(memory).await;
    records.push(PracticeRecord::new(item_id, knowledge_point, difficulty, correct));
    if records.len() > MAX_RECORDS {
        records = records.split_off(records.len() - MAX_RECORDS);
    }
    let content = serde_json::to_string(&records).unwrap_or_else(|_| "[]".into());
    let _ = memory.save(&path, &content).await;
}

/// 近期（近 MASTERED_WINDOW_DAYS 天）已答对（已掌握）的题目标识集合。
pub async fn recent_mastered(memory: &MemoryHandle) -> Vec<String> {
    let records = load_history(memory).await;
    let cutoff = Utc::now() - chrono::Duration::days(MASTERED_WINDOW_DAYS as i64);
    records
        .iter()
        .filter(|r| r.correct)
        .filter(|r| {
            DateTime::parse_from_rfc3339(&r.created_at)
                .map(|t| t.with_timezone(&Utc) >= cutoff)
                .unwrap_or(true)
        })
        .map(|r| r.item_id.clone())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::kernel::plugin::services::MemoryService;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeMemory {
        entries: Mutex<HashMap<String, String>>,
    }

    #[async_trait::async_trait]
    impl MemoryService for FakeMemory {
        async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), crate::kernel::plugin::services::MemoryError> {
            self.entries
                .lock()
                .expect("poisoned")
                .insert(path.as_str(), content.to_string());
            Ok(())
        }

        async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, crate::kernel::plugin::services::MemoryError> {
            match path {
                Some(p) => {
                    let entries = self.entries.lock().expect("poisoned");
                    match entries.get(&p.as_str()) {
                        Some(content) => Ok(MemoryView::Entry {
                            path: p.clone(),
                            content: content.clone(),
                        }),
                        None => Err(crate::kernel::plugin::services::MemoryError::NotFound(
                            p.as_str(),
                        )),
                    }
                }
                None => Ok(MemoryView::Listing(
                    self.entries.lock().expect("poisoned").keys().cloned().collect(),
                )),
            }
        }

        async fn remove(&self, path: &MemoryPath) -> Result<(), crate::kernel::plugin::services::MemoryError> {
            self.entries.lock().expect("poisoned").remove(&path.as_str());
            Ok(())
        }
    }

    fn handle() -> MemoryHandle {
        MemoryHandle::new(Arc::new(FakeMemory::default()))
    }

    use std::sync::Arc;

    #[tokio::test]
    async fn record_and_reload_roundtrip() {
        let memory = handle();
        record_attempt(&memory, "triangle_sss", "三角形全等判定", Difficulty::Basic, true).await;
        record_attempt(&memory, "abs_evaluate", "绝对值", Difficulty::Variant, false).await;
        let mastered = recent_mastered(&memory).await;
        assert_eq!(mastered, vec!["triangle_sss".to_string()]);
    }

    #[tokio::test]
    async fn missing_key_is_empty() {
        let memory = handle();
        assert!(recent_mastered(&memory).await.is_empty());
    }

    #[tokio::test]
    async fn caps_record_count() {
        let memory = handle();
        for i in 0..MAX_RECORDS + 10 {
            record_attempt(&memory, &format!("item_{i}"), "知识点", Difficulty::Basic, true).await;
        }
        let mastered = recent_mastered(&memory).await;
        assert_eq!(mastered.len(), MAX_RECORDS);
        assert!(!mastered.contains(&"item_0".to_string()));
        assert!(mastered.contains(&format!("item_{}", MAX_RECORDS + 9)));
    }
}
