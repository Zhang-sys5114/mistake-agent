//! exam 插件：阶段性考核组卷（场景四入口，按薄弱知识点 + 变式模板组卷）。

use std::collections::HashMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::dispatch::ToolCallContext;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};
use crate::kernel::services::{MistakeFilter, ServiceId, StorageHandle};
use crate::plugin::practice::{Difficulty, build_item};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ComposeParams {
    /// 限定学科（可传多个；缺省 = 全部学科）。
    pub subjects: Option<Vec<String>>,
    /// 题量（默认 5，上限 20）。
    pub count: Option<u32>,
    /// 建议用时分钟（默认按题量 × 10 推导，上限 240）。
    pub minutes: Option<u32>,
}

pub struct ExamPlugin;

impl UserPlugin for ExamPlugin {
    fn info() -> Info {
        Info {
            namespace: "exam".into(),
            requires: vec![ServiceId::Storage],
            tools: vec![ToolDef {
                name: "compose".into(),
                user_visible: true,
                title: Some("组一套试卷".into()),
                group: Some("学习".into()),
                description: "按错题本薄弱知识点自动组卷（限时、结构化题目与答案规格）。用法：exam::compose [学科 题数 分钟]".into(),
                params: schemars::schema_for!(ComposeParams),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:file-document".into()),
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
            "compose",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage.clone();
                Box::pin(async move { compose_handler(storage, params).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<ExamPlugin>()
}

async fn compose_handler(storage: StorageHandle, params: Value) -> Result<Value, ToolError> {
    let p: ComposeParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let count = p.count.unwrap_or(5).clamp(1, 20) as usize;
    let minutes = p.minutes.unwrap_or((count as u32) * 10).clamp(1, 240);

    let all = storage
        .list(&MistakeFilter::default())
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;
    let subjects: Vec<&str> = p
        .subjects
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|s| s.as_str())
        .collect();

    // 按知识点聚合错误次数（可限定学科），错得越多越优先。
    let mut by_point: HashMap<&str, usize> = HashMap::new();
    for m in all.iter().filter(|m| {
        !m.is_correct && (subjects.is_empty() || subjects.contains(&m.subject.as_str()))
    }) {
        *by_point.entry(m.knowledge_point.as_str()).or_default() += 1;
    }
    let mut points: Vec<(&str, usize)> = by_point.into_iter().collect();
    // 错误次数降序；次数相同按知识点字典序，保证输出确定性。
    points.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    points.truncate(count);

    let mut items = Vec::new();
    let mut covered = Vec::new();
    for (kp, wrong) in points {
        if let Some(item) = build_item(kp, Difficulty::Variant) {
            items.push(json!({
                "knowledge_point": item.knowledge_point,
                "template_id": item.template_id,
                "question": item.question_text,
                "answer_spec": item.answer_spec,
                "diagram_spec": item.diagram_spec,
            }));
            covered.push(json!({ "knowledge_point": kp, "wrong_count": wrong }));
        }
    }

    Ok(json!({
        "minutes": minutes,
        "count": items.len(),
        "items": items,
        "source_points": covered,
        "message": if items.is_empty() {
            "暂无可用模板覆盖的薄弱知识点（当前模板：三角形全等判定/绝对值/一般现在时三单）"
        } else {
            "已按薄弱知识点组卷，作答后可上传批改"
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::services::{Mistake, MistakeId, MistakePatch, MistakeStore, StorageError};
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

    fn mistake(subject: &str, kp: &str, correct: bool) -> Mistake {
        Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: subject.into(),
            knowledge_point: kp.into(),
            question: "q".into(),
            student_answer: "a".into(),
            reference_answer: Some("r".into()),
            is_correct: correct,
            analysis: "x".into(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn compose_uses_weakest_points_and_limits_count() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        store.items.lock().unwrap().extend([
            mistake("数学", "绝对值", false),
            mistake("数学", "绝对值", false),
            mistake("数学", "三角形全等判定", false),
            mistake("数学", "三角形全等判定", false),
            mistake("英语", "一般现在时三单", false),
            mistake("物理", "受力分析", false), // 无模板覆盖 → 不组进卷子
        ]);

        let out = compose_handler(handle, json!({ "count": 2 }))
            .await
            .unwrap();
        assert_eq!(out["count"], 2);
        let points: Vec<&str> = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["knowledge_point"].as_str().unwrap())
            .collect();
        assert!(points.contains(&"绝对值"));
        assert!(points.contains(&"三角形全等判定"));
        assert!(out["minutes"].as_u64().unwrap() >= 20);
    }

    #[tokio::test]
    async fn compose_subject_filter() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        store.items.lock().unwrap().extend([
            mistake("数学", "绝对值", false),
            mistake("英语", "一般现在时三单", false),
        ]);

        let out = compose_handler(handle, json!({ "subjects": ["英语"] }))
            .await
            .unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["items"][0]["knowledge_point"], "一般现在时三单");
    }

    #[tokio::test]
    async fn compose_empty_returns_message() {
        let handle = StorageHandle::new(Arc::new(FakeStore::default()));
        let out = compose_handler(handle, json!({})).await.unwrap();
        assert_eq!(out["count"], 0);
        assert!(out["message"].as_str().unwrap().contains("暂无"));
    }
}
