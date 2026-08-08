//! practice 插件：薄弱知识点定位（场景二前置步骤）。
//!
//! 基于错题本聚合近 N 天错题，按知识点统计错误次数并给出建议起点难度；
//! 供模型编排「定位漏洞 → 出题」链路（practice::gaps → practice::generate）。

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::contract::ToolError;
use crate::kernel::plugin::services::{MistakeFilter, StorageHandle};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct GapsParams {
    /// 学科过滤（可选，如「数学」；不填统计全部学科）。
    pub subject: Option<String>,
    /// 统计近 N 天（默认 30，上限 365）。
    pub days: Option<u64>,
    /// 返回薄弱知识点数量上限（默认 5，上限 10）。
    pub limit: Option<usize>,
}

/// 按错误次数给出建议起点难度：1 次 → 基础补漏；2~3 次 → 同类变式；4 次及以上 → 综合拔高。
fn suggested_difficulty(wrong_count: usize) -> &'static str {
    match wrong_count {
        0 | 1 => "basic",
        2..=3 => "variant",
        _ => "advanced",
    }
}

/// 聚合错题本：近 N 天错题按知识点分组，错误次数多者优先。
pub async fn gaps_handler(storage: StorageHandle, params: Value) -> Result<Value, ToolError> {
    let p: GapsParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let days = p.days.unwrap_or(30).clamp(1, 365);
    let limit = p.limit.unwrap_or(5).clamp(1, 10);

    let all = storage
        .list(&MistakeFilter {
            subject: p.subject.clone(),
            knowledge_point: None,
            is_correct: Some(false),
        })
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;

    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(days as i64);
    // 按知识点聚合：错误次数 + 最近一次错误时间 + 最近一次错误的学科。
    struct Point {
        wrong_count: usize,
        latest: chrono::DateTime<chrono::Utc>,
        subject: String,
    }
    let mut by_point: HashMap<String, Point> = HashMap::new();
    for m in all.iter().filter(|m| m.created_at >= cutoff) {
        let entry = by_point.entry(m.knowledge_point.clone()).or_insert(Point {
            wrong_count: 0,
            latest: m.created_at,
            subject: m.subject.clone(),
        });
        entry.wrong_count += 1;
        if m.created_at > entry.latest {
            entry.latest = m.created_at;
            entry.subject = m.subject.clone();
        }
    }

    let mut gaps: Vec<Value> = by_point
        .into_iter()
        .map(|(kp, pt)| {
            json!({
                "knowledge_point": kp,
                "subject": pt.subject,
                "wrong_count": pt.wrong_count,
                "latest_at": pt.latest.to_rfc3339(),
                "suggested_difficulty": suggested_difficulty(pt.wrong_count),
            })
        })
        .collect();
    gaps.sort_by(|a, b| {
        b["wrong_count"]
            .as_u64()
            .cmp(&a["wrong_count"].as_u64())
            .then_with(|| b["latest_at"].as_str().cmp(&a["latest_at"].as_str()))
    });
    gaps.truncate(limit);

    Ok(json!({
        "count": gaps.len(),
        "days": days,
        "gaps": gaps,
        "message": if gaps.is_empty() {
            "近 N 天错题本暂无薄弱知识点，先做一次作业批改积累错题吧。".to_string()
        } else {
            "已按错误次数排序；suggested_difficulty 为建议起点难度（basic 基础补漏 / variant 同类变式 / advanced 综合拔高）。".to_string()
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::plugin::services::{
        Mistake, MistakeId, MistakePatch, MistakeStore, StorageError, StorageHandle,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeStore {
        items: Mutex<Vec<Mistake>>,
    }

    #[async_trait::async_trait]
    impl MistakeStore for FakeStore {
        async fn save(&self, m: &Mistake) -> Result<MistakeId, StorageError> {
            self.items.lock().expect("poisoned").push(m.clone());
            Ok(m.id)
        }
        async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError> {
            Ok(self
                .items
                .lock()
                .expect("poisoned")
                .iter()
                .find(|m| m.id == *id)
                .cloned())
        }
        async fn list(&self, f: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
            let inner = self.items.lock().expect("poisoned");
            Ok(inner
                .iter()
                .filter(|m| {
                    f.subject
                        .as_deref()
                        .map(|s| m.subject == s)
                        .unwrap_or(true)
                        && f.knowledge_point
                            .as_deref()
                            .map(|k| m.knowledge_point == k)
                            .unwrap_or(true)
                        && f.is_correct.map(|c| m.is_correct == c).unwrap_or(true)
                })
                .cloned()
                .collect())
        }
        async fn update(&self, _id: &MistakeId, _p: &MistakePatch) -> Result<(), StorageError> {
            Err(StorageError::Internal("fake".into()))
        }
        async fn remove(&self, _id: &MistakeId) -> Result<(), StorageError> {
            Err(StorageError::Internal("fake".into()))
        }
    }

    fn mistake(subject: &str, kp: &str, correct: bool, days_ago: i64) -> Mistake {
        Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: subject.into(),
            knowledge_point: kp.into(),
            question: "q".into(),
            student_answer: "a".into(),
            reference_answer: Some("r".into()),
            is_correct: correct,
            analysis: "x".into(),
            created_at: chrono::Utc::now() - chrono::Duration::days(days_ago),
            pinned: false,
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn gaps_aggregates_ranks_and_suggests_difficulty() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        store.items.lock().unwrap().extend([
            mistake("数学", "绝对值", false, 1),
            mistake("数学", "绝对值", false, 1),
            mistake("数学", "全等判定", false, 2),
            mistake("数学", "全等判定", false, 2),
            mistake("数学", "全等判定", false, 2),
            mistake("英语", "三单", false, 3),
            mistake("英语", "三单", true, 3), // 答对的不计入
            mistake("数学", "老错点", false, 60), // 超出 30 天窗口
        ]);

        let out = gaps_handler(handle, json!({})).await.unwrap();
        assert_eq!(out["days"], 30);
        assert_eq!(out["gaps"][0]["knowledge_point"], "全等判定");
        assert_eq!(out["gaps"][0]["wrong_count"], 3);
        assert_eq!(out["gaps"][0]["suggested_difficulty"], "variant");
        assert_eq!(out["gaps"][1]["knowledge_point"], "绝对值");
        assert_eq!(out["gaps"][1]["wrong_count"], 2);
        assert_eq!(out["gaps"][1]["suggested_difficulty"], "variant");
        assert_eq!(out["gaps"][2]["knowledge_point"], "三单");
        assert_eq!(out["gaps"][2]["wrong_count"], 1);
        assert_eq!(out["gaps"][2]["suggested_difficulty"], "basic");
        assert_eq!(out["count"], 3);
    }

    #[tokio::test]
    async fn gaps_filters_by_subject() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        store.items.lock().unwrap().extend([
            mistake("数学", "绝对值", false, 1),
            mistake("英语", "三单", false, 1),
        ]);
        let out = gaps_handler(handle, json!({ "subject": "数学" })).await.unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["gaps"][0]["subject"], "数学");
    }

    #[tokio::test]
    async fn gaps_empty_store_returns_message() {
        let handle = StorageHandle::new(Arc::new(FakeStore::default()));
        let out = gaps_handler(handle, json!({})).await.unwrap();
        assert_eq!(out["count"], 0);
        assert!(out["message"].as_str().unwrap().contains("暂无"));
    }

    #[test]
    fn suggested_difficulty_tiers() {
        assert_eq!(suggested_difficulty(1), "basic");
        assert_eq!(suggested_difficulty(2), "variant");
        assert_eq!(suggested_difficulty(3), "variant");
        assert_eq!(suggested_difficulty(4), "advanced");
    }
}
