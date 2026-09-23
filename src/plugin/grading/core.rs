//! grading 核心实现：upload handler（归档模型判分结果）、进度播报。
//! 判分由主模型直接阅读上下文中的作业图片/PDF 正文完成（ADR-0046），本插件只落库。

use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::contract::ToolError;
use crate::kernel::events::Event;
use crate::kernel::plugin::services::{Mistake, MistakeId, MistakePatch, StorageHandle};

use super::params::{GetParams, RemoveManyParams, RemoveParams, UpdateParams, UploadParams};

fn parse_mistake_id(raw: &str) -> Result<MistakeId, ToolError> {
    uuid::Uuid::parse_str(raw)
        .map(MistakeId)
        .map_err(|e| ToolError::invalid_params(format!("非法错题 id：{e}")))
}

pub(crate) async fn get_handler(
    storage: StorageHandle,
    params: Value,
) -> Result<Value, ToolError> {
    let p: GetParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let id = parse_mistake_id(&p.id)?;
    let mistake = storage
        .get(&id)
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?
        .ok_or_else(|| ToolError::handler(format!("错题不存在：{}", p.id)))?;
    Ok(json!({ "mistake": mistake }))
}

pub(crate) async fn update_handler(
    storage: StorageHandle,
    params: Value,
) -> Result<Value, ToolError> {
    let p: UpdateParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let id = parse_mistake_id(&p.id)?;
    let patch = MistakePatch {
        subject: p.subject,
        knowledge_point: p.knowledge_point,
        question: p.question,
        student_answer: p.student_answer,
        reference_answer: p.reference_answer,
        analysis: p.analysis,
        is_correct: p.is_correct,
        pinned: p.pinned,
    };
    storage
        .update(&id, &patch)
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;
    let mistake = storage
        .get(&id)
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?
        .ok_or_else(|| ToolError::handler(format!("错题不存在：{}", p.id)))?;
    Ok(json!({ "mistake": mistake }))
}

pub(crate) async fn remove_handler(
    storage: StorageHandle,
    params: Value,
) -> Result<Value, ToolError> {
    let p: RemoveParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let id = parse_mistake_id(&p.id)?;
    storage
        .remove(&id)
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;
    Ok(json!({ "deleted": true, "id": id.to_string() }))
}

pub(crate) async fn remove_many_handler(
    storage: StorageHandle,
    params: Value,
) -> Result<Value, ToolError> {
    let p: RemoveManyParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let ids = p
        .ids
        .iter()
        .map(|raw| parse_mistake_id(raw))
        .collect::<Result<Vec<_>, _>>()?;
    let deleted = storage
        .remove_many(&ids)
        .await
        .map_err(|e| ToolError::handler(e.to_string()))?;
    Ok(json!({ "deleted": deleted }))
}

pub(crate) async fn upload_handler(
    ctx: &ToolCallContext,
    params: Value,
    storage: StorageHandle,
) -> Result<Value, ToolError> {
    let p: UploadParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    if p.items.is_empty() {
        return Err(ToolError::invalid_params("items 不能为空"));
    }
    emit_progress(ctx, "grading::upload", "正在归档错题…");

    let items = p.items;
    let mut wrong_count = 0usize;
    let mut archived = 0usize;
    for item in &items {
        if !item.correct {
            wrong_count += 1;
            let mistake = Mistake {
                id: MistakeId(uuid::Uuid::new_v4()),
                subject: item
                    .subject
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "未分类".into()),
                knowledge_point: item
                    .knowledge_point
                    .clone()
                    .unwrap_or_else(|| "未标注".into()),
                question: item.question.clone(),
                student_answer: item
                    .student_answer
                    .clone()
                    .unwrap_or_else(|| "（未作答）".into()),
                reference_answer: item
                    .reference_answer
                    .clone()
                    .filter(|s| !s.trim().is_empty()),
                is_correct: false,
                analysis: item.analysis.clone().unwrap_or_default(),
                created_at: chrono::Utc::now(),
                pinned: false,
                deleted_at: None,
            };
            match storage.save(&mistake).await {
                Ok(_) => archived += 1,
                Err(e) => {
                    return Err(ToolError::handler(format!("错题归档失败：{e}")));
                }
            }
        }
    }

    emit_progress(ctx, "grading::upload", "批改完成");
    Ok(json!({
        "total": items.len(),
        "correct_count": items.len() - wrong_count,
        "wrong_count": wrong_count,
        "archived_mistakes": archived,
        "items": items,
    }))
}

fn emit_progress(ctx: &ToolCallContext, entry: &str, message: &str) {
    ctx.events.emit(Event::ToolProgress {
        entry: entry.into(),
        message: message.into(),
        icon: Some("mdi:image-search".into()),
    });
}
