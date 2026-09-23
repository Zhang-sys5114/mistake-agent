//! grading 参数与结果 schema（upload/list 入参、判分结果 GradedItem）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// 归档入参（ADR-0046）：模型直接阅读上下文中的作业图片/PDF 正文后，把逐题判分结果传入，
/// 工具只负责把错题落库；不再由工具自己读图或再调模型。
#[derive(Debug, Clone, Deserialize)]
pub struct UploadParams {
    pub items: Vec<GradedItem>,
}

impl JsonSchema for UploadParams {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "UploadParams".into()
    }

    fn json_schema(_gen: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        // 内联 schema：避免 $defs/$ref（DeepSeek 工具参数不解析引用）。
        serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": graded_item_object_schema()
                }
            },
            "required": ["items"],
            "additionalProperties": false
        }))
        .unwrap()
    }
}

/// GradedItem 的内联对象 schema（供 UploadParams.items.items 复用）。
fn graded_item_object_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "number": {"type": ["string", "null"], "description": "题号"},
            "question": {"type": "string", "description": "题目原文（逐字保留，不要概括或漏小问）"},
            "student_answer": {"type": ["string", "null"], "description": "学生作答原文"},
            "subject": {"type": ["string", "null"], "description": "学科，如数学/英语/物理/化学/生物/语文，无法判断填 未分类"},
            "reference_answer": {"type": ["string", "null"], "description": "参考答案，可为 null"},
            "correct": {"type": "boolean", "description": "是否答对"},
            "score": {"type": ["number", "null"], "description": "得分"},
            "total": {"type": ["number", "null"], "description": "满分"},
            "knowledge_point": {"type": ["string", "null"], "description": "知识点"},
            "analysis": {"type": ["string", "null"], "description": "错因分析"}
        },
        "required": ["question", "correct"],
        "additionalProperties": false
    })
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListParams {
    pub subject: Option<String>,
    pub knowledge_point: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct GetParams {
    pub id: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct UpdateParams {
    pub id: String,
    pub subject: Option<String>,
    pub knowledge_point: Option<String>,
    pub question: Option<String>,
    pub student_answer: Option<String>,
    pub reference_answer: Option<Option<String>>,
    pub analysis: Option<String>,
    pub is_correct: Option<bool>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct RemoveParams {
    pub id: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct RemoveManyParams {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GradedItem {
    pub(crate) number: Option<String>,
    pub(crate) question: String,
    pub(crate) student_answer: Option<String>,
    pub(crate) subject: Option<String>,
    pub(crate) reference_answer: Option<String>,
    pub(crate) correct: bool,
    pub(crate) score: Option<f64>,
    pub(crate) total: Option<f64>,
    pub(crate) knowledge_point: Option<String>,
    pub(crate) analysis: Option<String>,
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
