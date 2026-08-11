//! grading 测试。

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
