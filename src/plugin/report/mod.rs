//! report 插件：多周期复盘（场景三入口，本期实现周复盘聚合统计）。

use std::collections::HashMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::{MistakeFilter, ServiceId, StorageHandle};
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct WeeklyParams {
    /// 统计近 N 天（默认 7，上限 365）。
    pub days: Option<u64>,
}

pub struct ReportPlugin;

impl UserPlugin for ReportPlugin {
    fn info() -> Info {
        Info {
            namespace: "report".into(),
            requires: vec![ServiceId::Storage],
            tools: vec![ToolDef {
                name: "weekly".into(),
                user_visible: true,
                title: Some("周复盘报告".into()),
                group: Some("学习".into()),
                description: "生成近 N 天的错题复盘报告：总数/正确率/新增错题/最薄弱知识点。用法：report::weekly [天数]".into(),
                params: schemars::schema_for!(WeeklyParams),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:chart-box".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        let storage = ctx
            .handles
            .storage()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Storage 句柄".into()))?;
        ctx.registrar.tool(
            "weekly",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage.clone();
                Box::pin(async move { weekly_handler(storage, params).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<ReportPlugin>()
}

async fn weekly_handler(storage: StorageHandle, params: Value) -> Result<Value, ToolError> {
    let p: WeeklyParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let days = p.days.unwrap_or(7).clamp(1, 365);
    let all = storage
        .list(&MistakeFilter::default())
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;

    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::days(days as i64);
    let recent: Vec<_> = all.iter().filter(|m| m.created_at >= cutoff).collect();

    let total = all.len();
    let correct_count = all.iter().filter(|m| m.is_correct).count();
    let wrong_count = total - correct_count;
    let correct_rate = if total == 0 {
        0.0
    } else {
        (correct_count as f64 / total as f64 * 1000.0).round() / 10.0
    };

    // 近 N 天错题按知识点聚合，错误次数最多的排前面。
    let mut by_point: HashMap<&str, usize> = HashMap::new();
    for m in recent.iter().filter(|m| !m.is_correct) {
        *by_point.entry(m.knowledge_point.as_str()).or_default() += 1;
    }
    let mut weakest: Vec<Value> = by_point
        .into_iter()
        .map(|(kp, count)| json!({ "knowledge_point": kp, "wrong_count": count }))
        .collect();
    weakest.sort_by(|a, b| b["wrong_count"].as_u64().cmp(&a["wrong_count"].as_u64()));
    weakest.truncate(3);

    Ok(json!({
        "as_of": now.to_rfc3339(),
        "days": days,
        "total_mistakes": total,
        "correct_count": correct_count,
        "wrong_count": wrong_count,
        "correct_rate": correct_rate,
        "recent_new": recent.len(),
        "weakest_points": weakest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::plugin::services::{
        Mistake, MistakeId, MistakePatch, MistakeStore, StorageError,
    };
    use std::sync::Mutex;

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
        async fn list(&self, _f: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
            Ok(self.items.lock().expect("poisoned").clone())
        }
        async fn update(&self, _id: &MistakeId, _p: &MistakePatch) -> Result<(), StorageError> {
            Err(StorageError::Internal("fake".into()))
        }
        async fn remove(&self, _id: &MistakeId) -> Result<(), StorageError> {
            Err(StorageError::Internal("fake".into()))
        }
    }

    fn mistake(kp: &str, correct: bool, days_ago: i64) -> Mistake {
        Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
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
    async fn weekly_aggregates_and_ranks_weakest() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        store.items.lock().unwrap().extend([
            mistake("绝对值", false, 1),
            mistake("绝对值", false, 1),
            mistake("全等", false, 2),
            mistake("三单", true, 3),
            mistake("老题", false, 30), // 超出 7 天窗口，不影响 weakest
        ]);

        let out = weekly_handler(handle, json!({ "days": 7 })).await.unwrap();
        assert_eq!(out["total_mistakes"], 5);
        assert_eq!(out["correct_count"], 1);
        assert_eq!(out["wrong_count"], 4);
        assert_eq!(out["correct_rate"], 20.0);
        assert_eq!(out["recent_new"], 4);
        assert_eq!(out["weakest_points"][0]["knowledge_point"], "绝对值");
        assert_eq!(out["weakest_points"][0]["wrong_count"], 2);
        assert_eq!(out["weakest_points"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn weekly_empty_store() {
        let handle = StorageHandle::new(Arc::new(FakeStore::default()));
        let out = weekly_handler(handle, json!({})).await.unwrap();
        assert_eq!(out["days"], 7);
        assert_eq!(out["total_mistakes"], 0);
        assert_eq!(out["correct_rate"], 0.0);
        assert_eq!(out["weakest_points"].as_array().unwrap().len(), 0);
    }
}
