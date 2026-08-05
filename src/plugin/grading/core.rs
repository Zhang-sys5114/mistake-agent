//! grading 核心实现：上传 handler（OCR → 判分 → 归档）、路径白名单、进度播报。

use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::contract::ToolError;
use crate::kernel::events::Event;
use crate::kernel::message::{Attachment, Message, MessageKind};
use crate::kernel::plugin::services::{
    AbortSignal, Mistake, MistakeId, ModelHandle, ModelKind, ModelRequest, ResponseFormat,
    StorageHandle,
};
use crate::kernel::prompt::{grading_system_prompt, ocr_prompt};

use super::params::{GradedItem, UploadParams};
pub(crate) async fn upload_handler(
    ctx: &ToolCallContext,
    params: Value,
    storage: StorageHandle,
    model: ModelHandle,
) -> Result<Value, ToolError> {
    let p: UploadParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let path = std::path::Path::new(&p.file);
    if !path.exists() {
        return Err(ToolError::handler(format!("文件不存在：{}", p.file)));
    }
    if !stage_path_allowed(path) {
        return Err(ToolError::handler(
            "出于安全考虑，只接受通过「作业」按钮选择的文件（已暂存到系统临时目录）。请重新选择文件。",
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| ToolError::handler(format!("读取失败：{e}")))?;
    // 暂存文件内容已读入内存，立即清理系统临时副本。
    let _ = std::fs::remove_file(path);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();

    emit_progress(ctx, "正在提取作业内容…");
    let content = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => {
            let mime = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                _ => "image/bmp",
            };
            ocr_image(&model, mime, &bytes, ctx).await?
        }
        "pdf" => extract_pdf_text(&bytes).await?,
        other => {
            return Err(ToolError::handler(format!(
                "不支持的文件类型：{other}（支持 png/jpg/jpeg/webp/bmp/pdf）"
            )));
        }
    };
    if content.trim().is_empty() {
        return Err(ToolError::handler(
            "未能识别到作业内容（扫描版 PDF 请拍照上传图片）",
        ));
    }

    emit_progress(ctx, "正在逐题判分…");
    let grading_text = grade_content(&model, &content, ctx).await?;
    let items: Vec<GradedItem> = parse_grading_json(&grading_text)?;

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
            };
            match storage.save(&mistake).await {
                Ok(_) => archived += 1,
                Err(e) => {
                    return Err(ToolError::handler(format!("错题归档失败：{e}")));
                }
            }
        }
    }

    emit_progress(ctx, "批改完成");
    Ok(json!({
        "total": items.len(),
        "correct_count": items.len() - wrong_count,
        "wrong_count": wrong_count,
        "archived_mistakes": archived,
        "items": items,
    }))
}

/// 安全白名单：只允许系统临时目录下 GUI 暂存的文件（mistake-agent- 前缀）。
/// canonicalize 防止符号链接逃逸到任意路径。
pub(crate) fn stage_path_allowed(path: &Path) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    let Some(name) = canonical.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    canonical.starts_with(std::env::temp_dir()) && name.starts_with("mistake-agent-")
}

/// OCR：视觉模型只做内容提取，不判分（用户明确要求）。
async fn ocr_image(
    model: &ModelHandle,
    mime: &str,
    bytes: &[u8],
    ctx: &ToolCallContext,
) -> Result<String, ToolError> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let attachments = vec![Attachment {
        mime: mime.into(),
        data_base64: b64,
    }];
    let mut msg = Message::user(ocr_prompt());
    if let MessageKind::User { attachments: a, .. } = &mut msg.kind {
        *a = attachments;
    }
    let request = ModelRequest::chat(ModelKind::Vision, vec![msg]);
    let response = model
        .complete(&request, &AbortSignal::new())
        .await
        .map_err(map_model_error)?;
    ctx.events.emit(Event::ToolProgress {
        entry: "grading::upload".into(),
        message: format!("OCR 完成（{} 字符）", response.text.chars().count()),
        icon: Some("mdi:upload".into()),
    });
    Ok(response.text)
}

async fn extract_pdf_text(bytes: &[u8]) -> Result<String, ToolError> {
    match pdf_extract::extract_text_from_mem(bytes) {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        Ok(_) => Err(ToolError::handler(
            "PDF 没有可提取的文字（可能是扫描版），请拍照上传图片",
        )),
        Err(e) => Err(ToolError::handler(format!("PDF 解析失败：{e}"))),
    }
}

/// 判分：主模型按 OCR 内容逐题批改，输出 JSON 数组。
async fn grade_content(
    model: &ModelHandle,
    ocr: &str,
    ctx: &ToolCallContext,
) -> Result<String, ToolError> {
    let system = Message::system(grading_system_prompt());
    let user = Message::user(format!("作业 OCR 内容：\n{ocr}\n请逐题批改。"));
    let mut request = ModelRequest::chat(ModelKind::Main, vec![system, user]);
    // 内联扁平数组 schema：避免 $defs/$ref（DeepSeek json_schema 端不解析引用）。
    let item_schema = serde_json::to_value(schemars::schema_for!(GradedItem)).unwrap_or_default();
    let schema = json!({
        "type": "array",
        "items": item_schema,
    });
    request.response_format = Some(ResponseFormat::JsonSchema {
        name: "graded_items".into(),
        schema: serde_json::to_value(schema).unwrap_or_default(),
    });
    request.reasoning_effort = Some("none".into());
    let response = model
        .complete(&request, &AbortSignal::new())
        .await
        .map_err(map_model_error)?;
    ctx.events.emit(Event::ToolProgress {
        entry: "grading::upload".into(),
        message: "判分完成".into(),
        icon: Some("mdi:upload".into()),
    });
    Ok(response.text)
}

fn parse_grading_json(text: &str) -> Result<Vec<GradedItem>, ToolError> {
    let trimmed = text.trim();
    if let Ok(items) = serde_json::from_str::<Vec<GradedItem>>(trimmed) {
        return Ok(items);
    }
    // 容灾：单对象（模型可能没按数组输出）。
    if let Ok(item) = serde_json::from_str::<GradedItem>(trimmed) {
        return Ok(vec![item]);
    }
    // 容灾：从文本中截取第一个 [ 到最后一个 ]。
    if let (Some(s), Some(e)) = (trimmed.find('['), trimmed.rfind(']')) {
        let slice = &trimmed[s..=e];
        if let Ok(items) = serde_json::from_str::<Vec<GradedItem>>(slice) {
            return Ok(items);
        }
    }
    // 容灾：截取第一个 { 到最后一个 }（单对象）。
    if let (Some(s), Some(e)) = (trimmed.find('{'), trimmed.rfind('}')) {
        let slice = &trimmed[s..=e];
        if let Ok(item) = serde_json::from_str::<GradedItem>(slice) {
            return Ok(vec![item]);
        }
    }
    Err(ToolError::handler(format!(
        "判分结果无法解析：{}",
        text.chars().take(200).collect::<String>()
    )))
}

fn map_model_error(e: crate::kernel::plugin::services::ModelError) -> ToolError {
    match e {
        crate::kernel::plugin::services::ModelError::Timeout => ToolError::timeout(),
        crate::kernel::plugin::services::ModelError::Cancelled => ToolError::aborted(),
        other if other.is_systemic() => ToolError::model_unavailable(other.to_string()),
        other => ToolError::handler(other.to_string()),
    }
}

fn emit_progress(ctx: &ToolCallContext, message: &str) {
    ctx.events.emit(Event::ToolProgress {
        entry: "grading::upload".into(),
        message: message.into(),
        icon: Some("mdi:upload".into()),
    });
}
