//! tracking 插件：长效追踪（场景五入口，掌握度 + 7/14/30 天重测计划 + 记忆快照）。

use std::collections::HashMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::{
    MemoryError, MemoryHandle, MemoryPath, MistakeFilter, ServiceId, StorageHandle,
};
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CheckinParams {
    /// 限定学科（可选）。
    pub subject: Option<String>,
}

pub struct TrackingPlugin;

impl UserPlugin for TrackingPlugin {
    fn info() -> Info {
        Info {
            namespace: "tracking".into(),
            requires: vec![ServiceId::Storage, ServiceId::Memory],
            tools: vec![ToolDef {
                name: "checkin".into(),
                user_visible: true,
                title: Some("掌握度检查".into()),
                group: Some("学习".into()),
                description: "生成掌握度快照与重测计划（weak 7 天 / normal 14 天 / mastered 30 天），并写入跨会话记忆。用法：tracking::checkin [学科]"
                    .into(),
                params: schemars::schema_for!(CheckinParams),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:calendar-check".into()),
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
        let memory = ctx
            .handles
            .memory()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Memory 句柄".into()))?;
        ctx.registrar.tool(
            "checkin",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage.clone();
                let memory = memory.clone();
                Box::pin(async move { checkin_handler(storage, memory, params).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<TrackingPlugin>()
}

async fn checkin_handler(
    storage: StorageHandle,
    memory: MemoryHandle,
    params: Value,
) -> Result<Value, ToolError> {
    let p: CheckinParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let filter = MistakeFilter {
        subject: p.subject.clone(),
        ..Default::default()
    };
    let all = storage
        .list(&filter)
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;

    // 按知识点聚合：总数/错误数 → 掌握度。
    let mut stats: HashMap<&str, (usize, usize)> = HashMap::new();
    for m in &all {
        let e = stats.entry(m.knowledge_point.as_str()).or_default();
        e.0 += 1;
        if !m.is_correct {
            e.1 += 1;
        }
    }

    let now = chrono::Utc::now();
    let mut mastery = Vec::new();
    let mut retest_plan = Vec::new();
    for (kp, (total, wrong)) in stats {
        let wrong_rate = if total == 0 {
            0.0
        } else {
            wrong as f64 / total as f64
        };
        let score = (100.0 - wrong_rate * 100.0).max(0.0).round() as u32;
        let level = if score < 50 {
            "weak"
        } else if score < 80 {
            "normal"
        } else {
            "mastered"
        };
        let due = match level {
            "weak" => "7",
            "normal" => "14",
            _ => "30",
        };
        mastery.push(json!({
            "knowledge_point": kp,
            "total": total,
            "wrong": wrong,
            "mastery": score,
            "level": level,
        }));
        retest_plan.push(json!({
            "knowledge_point": kp,
            "due_in_days": due,
        }));
    }

    // 记忆快照：无学科 → tracking/掌握度；有学科 → tracking/<学科>。
    let base = match &p.subject {
        Some(s) if !s.trim().is_empty() => format!("tracking/{}", s.trim()),
        _ => "tracking/掌握度".into(),
    };
    let snapshot = json!({
        "as_of": now.to_rfc3339(),
        "subject": p.subject.as_deref().unwrap_or("全部"),
        "mastery": mastery,
    });
    let mut saved_paths = Vec::new();
    for entry in mastery.iter() {
        let kp = entry["knowledge_point"].as_str().unwrap_or("未知");
        let path_str = format!("{base}/{kp}");
        if let Ok(path) = MemoryPath::parse(&path_str) {
            let content = serde_json::to_string(entry).unwrap_or_default();
            if memory.save(&path, &content).await.is_ok() {
                saved_paths.push(path.as_str());
            }
        }
    }
    if let Ok(base_path) = MemoryPath::parse(&base) {
        let content = serde_json::to_string_pretty(&snapshot).unwrap_or_default();
        memory
            .save(&base_path, &content)
            .await
            .map_err(map_memory_error)?;
        saved_paths.insert(0, base_path.as_str());
    }

    Ok(json!({
        "as_of": now.to_rfc3339(),
        "subject": p.subject,
        "mastery": mastery,
        "retest_plan": retest_plan,
        "snapshot_paths": saved_paths,
    }))
}

fn map_memory_error(e: MemoryError) -> ToolError {
    match e {
        MemoryError::InvalidPath(m) => ToolError::invalid_params(m),
        MemoryError::NotFound(m) => ToolError::handler(format!("记忆条目不存在：{m}")),
        MemoryError::Io(m) => ToolError::handler(format!("记忆读写失败：{m}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::plugin::services::{
        MemoryPath, MemoryService, MemoryView, Mistake, MistakeId, MistakePatch, MistakeStore,
        StorageError,
    };
    use std::collections::BTreeMap;
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

    #[derive(Default)]
    struct FakeMemory {
        entries: Mutex<BTreeMap<String, String>>,
    }

    #[async_trait::async_trait]
    impl crate::kernel::plugin::services::MemoryService for FakeMemory {
        async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError> {
            self.entries
                .lock()
                .expect("poisoned")
                .insert(path.as_str(), content.to_string());
            Ok(())
        }
        async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
            let entries = self.entries.lock().expect("poisoned");
            match path {
                None => Ok(MemoryView::Listing(entries.keys().cloned().collect())),
                Some(p) => entries
                    .get(&p.as_str())
                    .cloned()
                    .map(|content| MemoryView::Entry {
                        path: p.clone(),
                        content,
                    })
                    .ok_or_else(|| MemoryError::NotFound(p.as_str())),
            }
        }
        async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError> {
            let prefix = format!("{}/", path.as_str());
            let mut entries = self.entries.lock().expect("poisoned");
            let keys: Vec<String> = entries
                .keys()
                .filter(|k| *k == &path.as_str() || k.starts_with(&prefix))
                .cloned()
                .collect();
            if keys.is_empty() {
                return Err(MemoryError::NotFound(path.as_str()));
            }
            for k in keys {
                entries.remove(&k);
            }
            Ok(())
        }
    }

    fn mistake(kp: &str, correct: bool) -> Mistake {
        Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: kp.into(),
            question: "q".into(),
            student_answer: "a".into(),
            reference_answer: Some("r".into()),
            is_correct: correct,
            analysis: "x".into(),
            created_at: chrono::Utc::now(),
            pinned: false,
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn checkin_computes_levels_and_saves_snapshot() {
        let store = Arc::new(FakeStore::default());
        let mem = Arc::new(FakeMemory::default());
        store.items.lock().unwrap().extend([
            mistake("绝对值", false),
            mistake("绝对值", false),
            mistake("绝对值", true),
            mistake("全等", false),
        ]);
        let out = checkin_handler(
            StorageHandle::new(store.clone()),
            MemoryHandle::new(mem.clone()),
            json!({"subject": "数学"}),
        )
        .await
        .unwrap();

        assert_eq!(out["subject"], "数学");
        let mastery = out["mastery"].as_array().unwrap();
        assert_eq!(mastery.len(), 2);
        let abs = mastery
            .iter()
            .find(|m| m["knowledge_point"] == "绝对值")
            .unwrap();
        assert_eq!(abs["mastery"], 33.0);
        assert_eq!(abs["level"], "weak");
        let plan = out["retest_plan"].as_array().unwrap();
        assert!(plan.iter().any(|p| p["due_in_days"] == "7"));

        // 快照应已写入记忆（tracking/数学/...）。
        let listing = mem.show(None).await.unwrap();
        let names = match listing {
            MemoryView::Listing(names) => names,
            _ => unreachable!(),
        };
        assert!(names.iter().any(|n| n.starts_with("tracking/数学/")));
        assert!(names.iter().any(|n| n == "tracking/数学"));
    }

    #[tokio::test]
    async fn checkin_empty_store_writes_base_snapshot() {
        let mem = Arc::new(FakeMemory::default());
        let out = checkin_handler(
            StorageHandle::new(Arc::new(FakeStore::default())),
            MemoryHandle::new(mem.clone()),
            json!({}),
        )
        .await
        .unwrap();
        assert_eq!(out["mastery"].as_array().unwrap().len(), 0);
        assert!(out["snapshot_paths"][0] == "tracking/掌握度");
    }
}
