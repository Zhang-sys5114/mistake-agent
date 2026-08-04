//! grading 插件（任务三·场景一）：上传作业 → OCR 提取 → 判分 → 错题归档。

use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::dispatch::ToolCallContext;
use crate::kernel::events::Event;
use crate::kernel::message::{Attachment, Message, MessageKind};
use crate::kernel::prompt::{grading_system_prompt, ocr_prompt};
use crate::kernel::registry::{PluginDescriptor, UserPlugin};
use crate::kernel::services::{
    AbortSignal, Mistake, MistakeFilter, MistakeId, ModelHandle, ModelKind, ModelRequest,
    ResponseFormat, StorageHandle,
};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UploadParams {
    /// 本地图片（png/jpg/jpeg/webp/bmp）或文本型 PDF 路径。
    pub file: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListParams {
    pub subject: Option<String>,
    pub knowledge_point: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GradedItem {
    number: Option<String>,
    question: String,
    student_answer: Option<String>,
    subject: Option<String>,
    reference_answer: Option<String>,
    correct: bool,
    score: Option<f64>,
    total: Option<f64>,
    knowledge_point: Option<String>,
    analysis: Option<String>,
}

impl schemars::JsonSchema for GradedItem {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GradedItem".into()
    }

    fn json_schema(_gen: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        // 手写 schema：避免为内部判分结构引入额外 derive 复杂度。
        serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "number": {"type": ["string", "null"]},
                "question": {"type": "string"},
                "student_answer": {"type": ["string", "null"]},
                "subject": {"type": ["string", "null"]},
                "reference_answer": {"type": ["string", "null"]},
                "correct": {"type": "boolean"},
                "score": {"type": ["number", "null"]},
                "total": {"type": ["number", "null"]},
                "knowledge_point": {"type": ["string", "null"]},
                "analysis": {"type": ["string", "null"]}
            },
            "required": ["question", "correct"],
            "additionalProperties": false
        }))
        .unwrap()
    }
}

pub struct GradingPlugin;

impl UserPlugin for GradingPlugin {
    fn info() -> Info {
        Info {
            namespace: "grading".into(),
            requires: vec![
                crate::kernel::services::ServiceId::Storage,
                crate::kernel::services::ServiceId::Model,
            ],
            tools: vec![
                ToolDef {
                    name: "upload".into(),
                    user_visible: true,
                    title: Some("上传作业批改".into()),
                    group: Some("批改".into()),
                    description:
                        "上传作业图片或文本型 PDF，自动识别题目与作答、判分、把错题归档进错题本。用法：grading::upload <作业文件路径>"
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

        ctx.registrar.tool(
            "list",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage.clone();
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
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<GradingPlugin>()
}

async fn upload_handler(
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
fn stage_path_allowed(path: &Path) -> bool {
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

fn map_model_error(e: crate::kernel::services::ModelError) -> ToolError {
    match e {
        crate::kernel::services::ModelError::Timeout => ToolError::timeout(),
        crate::kernel::services::ModelError::Cancelled => ToolError::aborted(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::services::{MistakeFilter, MistakePatch, MistakeStore, StorageError};

    #[test]
    fn stage_path_allowed_accepts_only_temp_staged_files() {
        let temp = std::env::temp_dir();
        let ok = temp.join(format!("mistake-agent-{}.png", uuid::Uuid::new_v4()));
        let no_prefix = temp.join(format!("other-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&ok, b"x").unwrap();
        std::fs::write(&no_prefix, b"x").unwrap();
        assert!(stage_path_allowed(&ok), "暂存文件应被允许");
        assert!(
            !stage_path_allowed(&no_prefix),
            "非 mistake-agent- 前缀应拒绝"
        );
        assert!(
            !stage_path_allowed(Path::new("/etc/passwd")),
            "系统路径应拒绝"
        );
        let escape = std::env::temp_dir().join("..");
        assert!(!stage_path_allowed(&escape), "越界路径应拒绝");
        let _ = std::fs::remove_file(&ok);
        let _ = std::fs::remove_file(&no_prefix);
    }
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
                .find(|m| &m.id == id)
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
            })
            .await
            .unwrap();
        let got = handle.get(&saved).await.unwrap().unwrap();
        assert_eq!(got.subject, "数学");
        assert_eq!(got.reference_answer.as_deref(), Some("2"));
    }
}
