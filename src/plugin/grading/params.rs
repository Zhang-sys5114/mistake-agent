//! grading 参数与结果 schema（upload/list 入参、判分结果 GradedItem）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

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
