//! practice 插件：LLM 自由出题（模板未命中时的兜底路径，P1 智能出题核心）。
//!
//! 与 docs/variants.md 协议一致：结构化规格（question_text / answer_spec /
//! diagram_spec），题目、答案、图纸三者同源；确定性模板仍是优先路径。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::kernel::contract::ToolError;
use crate::kernel::message::Message;
use crate::kernel::plugin::services::{
    AbortSignal, ModelHandle, ModelKind, ModelRequest, ResponseFormat,
};
use crate::kernel::prompt::practice_generate_system_prompt;
use crate::plugin::vision::map_model_error;

use super::templates::{Difficulty, PracticeItem};

/// 模型自由出题结果：沿用 docs/variants.md 的结构化规格（不包含 template_id，
/// 该字段由调用方以固定值 llm_freeform 落盘，标识自由出题路径）。
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GeneratedItem {
    pub knowledge_point: String,
    pub question_text: String,
    pub answer_spec: String,
    #[serde(default)]
    pub diagram_spec: Option<Value>,
}

/// 难度 → 出题提示中的语义描述（与 docs/variants.md 分层定义一致）。
fn difficulty_label(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Basic => "basic（基础：直接套用公式/定理）",
        Difficulty::Variant => "variant（同类变式：条件隐藏或逆用）",
        Difficulty::Advanced => "advanced（综合拔高：多步组合、辅助线、跨知识点联动）",
    }
}

/// 模板未命中时走 LLM 生成：json_schema 强约束 + 容灾解析。
pub async fn model_generate(
    model: &ModelHandle,
    knowledge_point: &str,
    difficulty: Difficulty,
) -> Result<PracticeItem, ToolError> {
    let system = Message::system(practice_generate_system_prompt());
    let user = Message::user(format!(
        "知识点：{}\n难度：{}",
        knowledge_point.trim(),
        difficulty_label(difficulty)
    ));
    let mut request = ModelRequest::chat(ModelKind::Main, vec![system, user]);
    request.response_format = Some(ResponseFormat::JsonSchema {
        name: "practice_generate".into(),
        schema: serde_json::to_value(schemars::schema_for!(GeneratedItem)).unwrap_or_default(),
    });
    request.reasoning_effort = Some("none".into());
    let response = model
        .complete(&request, &AbortSignal::new())
        .await
        .map_err(map_model_error)?;
    let item = parse_generate_json(&response.text)?;
    Ok(PracticeItem {
        knowledge_point: item.knowledge_point.trim().to_string(),
        template_id: "llm_freeform".into(),
        difficulty,
        question_text: item.question_text,
        answer_spec: item.answer_spec,
        diagram_spec: item.diagram_spec,
    })
}

/// 容灾解析：先整段解析，失败则截取第一个 { 到最后一个 }（模型偶尔带前后缀）。
fn parse_generate_json(text: &str) -> Result<GeneratedItem, ToolError> {
    let trimmed = text.trim();
    if let Ok(r) = serde_json::from_str::<GeneratedItem>(trimmed) {
        return Ok(r);
    }
    if let (Some(s), Some(e)) = (trimmed.find('{'), trimmed.rfind('}')) {
        let slice = &trimmed[s..=e];
        if let Ok(r) = serde_json::from_str::<GeneratedItem>(slice) {
            return Ok(r);
        }
    }
    Err(ToolError::handler(format!(
        "模型出题结果无法解析：{}",
        text.chars().take(200).collect::<String>()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_label_maps_all_tiers() {
        assert!(difficulty_label(Difficulty::Basic).contains("basic"));
        assert!(difficulty_label(Difficulty::Variant).contains("variant"));
        assert!(difficulty_label(Difficulty::Advanced).contains("advanced"));
    }

    #[test]
    fn parse_accepts_plain_json() {
        let item = parse_generate_json(
            r#"{"knowledge_point":"一元二次方程","question_text":"解方程 $x^2-3x+2=0$。","answer_spec":"$x=1$ 或 $x=2$","diagram_spec":null}"#,
        )
        .unwrap();
        assert_eq!(item.knowledge_point, "一元二次方程");
        assert!(item.diagram_spec.is_none());
    }

    #[test]
    fn parse_recovers_fenced_json() {
        let item = parse_generate_json(
            r#"好的，题目如下：
```json
{"knowledge_point":"圆与切线","question_text":"如图，PA 是圆 O 的切线，A 为切点。","answer_spec":"PA⊥OA","diagram_spec":{"points":{"O":[0,0],"A":[3,0]},"objects":[{"type":"circle","center":"O","radius":3},{"type":"segment","ends":["O","A"]}],"labels":["O","A"]}}
```"#,
        )
        .unwrap();
        assert_eq!(item.knowledge_point, "圆与切线");
        let spec = item.diagram_spec.unwrap();
        assert_eq!(spec["points"]["O"][0], 0);
        assert_eq!(spec["objects"].as_array().unwrap()[0]["type"], "circle");
    }

    #[test]
    fn parse_rejects_garbage() {
        let err = parse_generate_json("抱歉，我无法出题。").unwrap_err();
        assert!(err.message.contains("无法解析"));
    }
}
