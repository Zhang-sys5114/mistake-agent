//! grading 插件（任务三·场景一）：上传作业 → 读图（vision::read）→ 判分 → 错题归档。
//!
//! 插件信息：namespace = grading，requires = [Storage, Model]
//! tools = [upload（判分归档）, list（错题本）]；看图（vision::read）独立成 vision 插件。
//! 实现拆分（Linux 内核风格）：`params.rs` 参数与结果 schema / `core.rs` 判分·归档 handler

use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, CommandDef, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::MistakeFilter;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};
mod core;
mod params;

use core::{
    get_handler, remove_handler, remove_many_handler, update_handler, upload_handler,
};
use params::{GetParams, ListParams, RemoveManyParams, RemoveParams, UpdateParams, UploadParams};

pub struct GradingPlugin;

impl UserPlugin for GradingPlugin {
    fn info() -> Info {
        Info {
            namespace: "grading".into(),
            // 默认懒加载：工具列表（model_tools）读 info 声明，第一轮即可见；
            // 模型 wire 调用命中未加载插件时由 resolve_wire 触发懒加载（ADR-0003）。
            requires: vec![
                crate::kernel::plugin::services::ServiceId::Storage,
                crate::kernel::plugin::services::ServiceId::Model,
            ],
            tools: vec![
                ToolDef {
                    name: "upload".into(),
                    user_visible: true,
                    title: Some("上传作业批改".into()),
                    group: Some("批改".into()),
                    description:
                        "上传作业图片或文本型 PDF，自动识别题目与作答、判分、把错题归档进错题本。文件由应用「选择作业文件」按钮上传后自动暂存，file 参数使用消息里给出的暂存路径。"
                            .into(),
                    params: schemars::schema_for!(UploadParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: Some(180),
                    icon: Some("mdi:upload".into()),
                },
                ToolDef {
                    name: "list".into(),
                    user_visible: true,
                    title: Some("查看错题本".into()),
                    group: Some("批改".into()),
                    description: "列出错题本中的错题（可按学科/知识点过滤）。用法：grading::list [学科] [知识点]".into(),
                    params: schemars::schema_for!(ListParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: Some("mdi:format-list-bulleted".into()),
                },
            ],
            commands: vec![
                CommandDef {
                    name: "get".into(),
                    user_visible: false,
                    title: Some("查看错题详情".into()),
                    group: Some("错题本".into()),
                    description: "按 id 获取单条错题详情，供详情页和追问使用。".into(),
                    params: schemars::schema_for!(GetParams),
                    icon: Some("mdi:card-text-outline".into()),
                },
                CommandDef {
                    name: "update".into(),
                    user_visible: false,
                    title: Some("编辑错题".into()),
                    group: Some("错题本".into()),
                    description: "编辑错题字段；置顶传 pinned，标记已掌握传 is_correct=true。".into(),
                    params: schemars::schema_for!(UpdateParams),
                    icon: Some("mdi:pencil-outline".into()),
                },
                CommandDef {
                    name: "remove".into(),
                    user_visible: false,
                    title: Some("删除错题".into()),
                    group: Some("错题本".into()),
                    description: "软删除单条错题，列表不再展示，数据仍保留。".into(),
                    params: schemars::schema_for!(RemoveParams),
                    icon: Some("mdi:delete-outline".into()),
                },
                CommandDef {
                    name: "remove_many".into(),
                    user_visible: false,
                    title: Some("批量删除错题".into()),
                    group: Some("错题本".into()),
                    description: "按 id 列表批量软删除，支持全选后整页删除。".into(),
                    params: schemars::schema_for!(RemoveManyParams),
                    icon: Some("mdi:delete-sweep-outline".into()),
                },
            ],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        let storage = ctx
            .handles
            .storage()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Storage 句柄".into()))?;
        let model = ctx
            .handles
            .model()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Model 句柄".into()))?;

        let storage_upload = storage.clone();
        ctx.registrar.tool(
            "upload",
            std::sync::Arc::new(move |call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_upload.clone();
                let model = model.clone();
                Box::pin(async move { upload_handler(call_ctx, params, storage, model).await })
            }),
        )?;

        let storage_list = storage.clone();
        ctx.registrar.tool(
            "list",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_list.clone();
                Box::pin(async move {
                    let filter: ListParams = serde_json::from_value(params)
                        .map_err(|e| ToolError::invalid_params(e.to_string()))?;
                    let filter = MistakeFilter {
                        subject: filter.subject,
                        knowledge_point: filter.knowledge_point,
                        is_correct: None,
                    };
                    let mistakes = storage
                        .list(&filter)
                        .await
                        .map_err(|e| ToolError::handler(e.to_string()))?;
                    Ok(json!({
                        "count": mistakes.len(),
                        "mistakes": mistakes,
                    }))
                })
            }),
        )?;

        let storage_get = storage.clone();
        ctx.registrar.command(
            "get",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_get.clone();
                Box::pin(async move { get_handler(storage, params).await })
            }),
        )?;

        let storage_update = storage.clone();
        ctx.registrar.command(
            "update",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_update.clone();
                Box::pin(async move { update_handler(storage, params).await })
            }),
        )?;

        let storage_remove = storage.clone();
        ctx.registrar.command(
            "remove",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_remove.clone();
                Box::pin(async move { remove_handler(storage, params).await })
            }),
        )?;

        let storage_remove_many = storage.clone();
        ctx.registrar.command(
            "remove_many",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_remove_many.clone();
                Box::pin(async move { remove_many_handler(storage, params).await })
            }),
        )?;

        Ok(())
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<GradingPlugin>()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::kernel::plugin::services::{
        Mistake, MistakeFilter, MistakeId, MistakePatch, MistakeStore, StorageError, StorageHandle,
    };
    use crate::plugin::grading::params::GradedItem;
    use std::sync::Arc;

    /// 内存假错题本：插件单测不落盘、不调真实 API。
    #[derive(Default)]
    struct FakeStore {
        items: std::sync::Mutex<Vec<Mistake>>,
    }

    #[async_trait::async_trait]
    impl MistakeStore for FakeStore {
        async fn save(&self, m: &Mistake) -> Result<MistakeId, StorageError> {
            let id = m.id;
            self.items.lock().expect("poisoned").push(m.clone());
            Ok(id)
        }
        async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError> {
            Ok(self
                .items
                .lock()
                .expect("poisoned")
                .iter()
                .find(|m| &m.id == id && m.deleted_at.is_none())
                .cloned())
        }
        async fn list(&self, f: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
            Ok(self
                .items
                .lock()
                .expect("poisoned")
                .iter()
                .filter(|m| {
                    f.subject.as_ref().is_none_or(|s| &m.subject == s)
                        && f.knowledge_point
                            .as_ref()
                            .is_none_or(|k| &m.knowledge_point == k)
                        && f.is_correct.is_none_or(|c| m.is_correct == c)
                        && m.deleted_at.is_none()
                })
                .cloned()
                .collect())
        }
        async fn update(&self, id: &MistakeId, p: &MistakePatch) -> Result<(), StorageError> {
            let mut items = self.items.lock().expect("poisoned");
            let m = items
                .iter_mut()
                .find(|m| m.id == *id)
                .ok_or_else(|| StorageError::MistakeNotFound(id.to_string()))?;
            if m.deleted_at.is_some() {
                return Err(StorageError::MistakeNotFound(id.to_string()));
            }
            if let Some(s) = &p.subject {
                m.subject = s.clone();
            }
            if let Some(k) = &p.knowledge_point {
                m.knowledge_point = k.clone();
            }
            if let Some(q) = &p.question {
                m.question = q.clone();
            }
            if let Some(s) = &p.student_answer {
                m.student_answer = s.clone();
            }
            if let Some(r) = &p.reference_answer {
                m.reference_answer = r.clone();
            }
            if let Some(a) = &p.analysis {
                m.analysis = a.clone();
            }
            if let Some(c) = p.is_correct {
                m.is_correct = c;
            }
            if let Some(pinned) = p.pinned {
                m.pinned = pinned;
            }
            Ok(())
        }
        async fn remove(&self, id: &MistakeId) -> Result<(), StorageError> {
            let mut items = self.items.lock().expect("poisoned");
            let m = items
                .iter_mut()
                .find(|m| m.id == *id)
                .ok_or_else(|| StorageError::MistakeNotFound(id.to_string()))?;
            if m.deleted_at.is_none() {
                m.deleted_at = Some(chrono::Utc::now());
            }
            Ok(())
        }
    }

    #[test]
    fn graded_item_parses_subject_and_reference_answer() {
        let json = r#"{
            "number": "2",
            "question": "判断向量组相关性",
            "student_answer": "未作答",
            "subject": "线性代数",
            "reference_answer": "线性相关",
            "correct": false,
            "score": 0,
            "total": 5,
            "knowledge_point": "向量组的线性相关性",
            "analysis": "未作答"
        }"#;
        let item: GradedItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.subject.as_deref(), Some("线性代数"));
        assert_eq!(item.reference_answer.as_deref(), Some("线性相关"));
        assert!(!item.correct);
    }

    #[test]
    fn graded_item_schema_contains_new_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(GradedItem)).unwrap();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("subject"));
        assert!(props.contains_key("reference_answer"));
        assert!(props.contains_key("student_answer"));
    }

    #[tokio::test]
    async fn list_handler_filters_by_subject() {
        // 与生产闭包同一逻辑：schema 解析 → MistakeFilter → storage.list。
        async fn list_handler(storage: StorageHandle, params: Value) -> Result<Value, ToolError> {
            let filter: ListParams = serde_json::from_value(params)
                .map_err(|e| ToolError::invalid_params(e.to_string()))?;
            let filter = MistakeFilter {
                subject: filter.subject,
                knowledge_point: filter.knowledge_point,
                is_correct: None,
            };
            let mistakes = storage
                .list(&filter)
                .await
                .map_err(|e| ToolError::handler(e.to_string()))?;
            Ok(json!({
                "count": mistakes.len(),
                "mistakes": mistakes,
            }))
        }

        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        let mut m = Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: "绝对值".into(),
            question: "|-3| = ?".into(),
            student_answer: "-3".into(),
            reference_answer: Some("3".into()),
            is_correct: false,
            analysis: "负数的绝对值".into(),
            created_at: chrono::Utc::now(),
            pinned: false,
            deleted_at: None,
        };
        handle.save(&m).await.unwrap();
        m.id = MistakeId(uuid::Uuid::new_v4());
        m.subject = "英语".into();
        m.knowledge_point = "一般现在时".into();
        handle.save(&m).await.unwrap();

        let out = list_handler(handle, json!({ "subject": "数学" }))
            .await
            .unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["mistakes"][0]["subject"], "数学");
    }

    #[tokio::test]
    async fn upload_archives_subject_and_reference_answer() {
        // 只测归档映射：item 带 subject/reference_answer 时错题不再恒为"未分类"。
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        let item = GradedItem {
            number: Some("1".into()),
            question: "1+1=?".into(),
            student_answer: Some("3".into()),
            subject: Some("数学".into()),
            reference_answer: Some("2".into()),
            correct: false,
            score: Some(0.0),
            total: Some(5.0),
            knowledge_point: Some("加法".into()),
            analysis: Some("算错".into()),
        };
        let saved = store
            .save(&Mistake {
                id: MistakeId(uuid::Uuid::new_v4()),
                subject: item.subject.clone().unwrap_or_else(|| "未分类".into()),
                knowledge_point: item.knowledge_point.clone().unwrap_or_default(),
                question: item.question.clone(),
                student_answer: item.student_answer.clone().unwrap_or_default(),
                reference_answer: item.reference_answer.clone(),
                is_correct: item.correct,
                analysis: item.analysis.clone().unwrap_or_default(),
                created_at: chrono::Utc::now(),
                pinned: false,
                deleted_at: None,
            })
            .await
            .unwrap();
        let got = handle.get(&saved).await.unwrap().unwrap();
        assert_eq!(got.subject, "数学");
        assert_eq!(got.reference_answer.as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn get_handler_returns_single_mistake() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        let m = Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: "绝对值".into(),
            question: "|-3| = ?".into(),
            student_answer: "-3".into(),
            reference_answer: Some("3".into()),
            is_correct: false,
            analysis: "符号错误".into(),
            created_at: chrono::Utc::now(),
            pinned: false,
            deleted_at: None,
        };
        let id = handle.save(&m).await.unwrap();

        let out = get_handler(handle, json!({ "id": id.to_string() }))
            .await
            .unwrap();
        assert_eq!(out["mistake"]["id"], id.to_string());
    }

    #[tokio::test]
    async fn update_handler_pins_and_marks_mastered() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        let m = Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: "绝对值".into(),
            question: "|-3| = ?".into(),
            student_answer: "-3".into(),
            reference_answer: Some("3".into()),
            is_correct: false,
            analysis: "符号错误".into(),
            created_at: chrono::Utc::now(),
            pinned: false,
            deleted_at: None,
        };
        let id = handle.save(&m).await.unwrap();

        let out = update_handler(
            handle.clone(),
            json!({ "id": id.to_string(), "pinned": true, "is_correct": true }),
        )
        .await
        .unwrap();
        assert_eq!(out["mistake"]["pinned"], true);
        assert_eq!(out["mistake"]["is_correct"], true);

        let stored = store
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .unwrap();
        assert!(stored.pinned);
        assert!(stored.is_correct);
    }

    #[tokio::test]
    async fn remove_many_handler_soft_deletes_selected() {
        let store = Arc::new(FakeStore::default());
        let handle = StorageHandle::new(store.clone());
        let mut m = Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: "绝对值".into(),
            question: "|-3| = ?".into(),
            student_answer: "-3".into(),
            reference_answer: Some("3".into()),
            is_correct: false,
            analysis: "符号错误".into(),
            created_at: chrono::Utc::now(),
            pinned: false,
            deleted_at: None,
        };
        let id1 = handle.save(&m).await.unwrap();
        m.id = MistakeId(uuid::Uuid::new_v4());
        let id2 = handle.save(&m).await.unwrap();

        let out = remove_many_handler(
            handle.clone(),
            json!({ "ids": [id1.to_string(), id2.to_string()] }),
        )
        .await
        .unwrap();
        assert_eq!(out["deleted"], 2);
        assert!(handle.list(&MistakeFilter::default()).await.unwrap().is_empty());
    }
}
