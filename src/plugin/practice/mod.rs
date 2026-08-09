//! practice 插件：分层变式练习（场景二入口：薄弱点定位 + 分层变式练习）。
//!
//! 插件信息：namespace = practice，requires = [Storage, Model]
//! tools = [generate（变式练习）, gaps（薄弱点定位）, check（练习答案批改）]
//! 实现拆分（Linux 内核风格）：`templates.rs` 模板库（题目/答案/图纸同源）；`gaps.rs` 薄弱点聚合；`check.rs` 答案批改

use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::{ModelHandle, ServiceId};
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

mod check;
mod generate;
mod gaps;
mod templates;

use check::{CheckParams, check_handler};
use generate::model_generate;
use gaps::{GapsParams, gaps_handler};
use templates::GenerateParams;
pub use templates::{Difficulty, PracticeItem, SUPPORTED_POINTS, build_item};

pub struct PracticePlugin;

impl UserPlugin for PracticePlugin {
    fn info() -> Info {
        Info {
            namespace: "practice".into(),
            requires: vec![ServiceId::Storage, ServiceId::Model],
            tools: vec![ToolDef {
                name: "generate".into(),
                user_visible: true,
                title: Some("生成变式练习".into()),
                group: Some("学习".into()),
                description:
                    "按知识点生成分层变式练习（基础/同类变式/综合拔高），几何题附带图纸规格。用法：practice::generate <知识点> [难度]".into(),
                params: schemars::schema_for!(GenerateParams),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:creation".into()),
            },
            ToolDef {
                name: "gaps".into(),
                user_visible: true,
                title: Some("薄弱点定位".into()),
                group: Some("学习".into()),
                description: "基于错题本聚合近 N 天薄弱知识点（按错误次数排序，含建议起点难度），用于定位知识漏洞后出题。用法：practice::gaps [学科] [天数] [数量]".into(),
                params: schemars::schema_for!(GapsParams),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:target".into()),
            },
            ToolDef {
                name: "check".into(),
                user_visible: true,
                title: Some("练习答案批改".into()),
                group: Some("学习".into()),
                description: "批改一道练习作答：参考答案可对拍时直接判分，否则由模型判分；答错自动回写错题本。用法：practice::check <题目> <学生答案> [参考答案] [学科] [知识点]".into(),
                params: schemars::schema_for!(CheckParams),
                policy: CallerPolicy::UserAndModel,
                timeout: Some(60),
                icon: Some("mdi:check-decagram".into()),
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
        let model = ctx
            .handles
            .model()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Model 句柄".into()))?;

        let model_for_generate = model.clone();
        ctx.registrar.tool(
            "generate",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let model = model_for_generate.clone();
                Box::pin(async move { generate_handler(model, params).await })
            }),
        )?;

        let storage_for_gaps = storage.clone();
        ctx.registrar.tool(
            "gaps",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_for_gaps.clone();
                Box::pin(async move { gaps_handler(storage, params).await })
            }),
        )?;

        let storage_for_check = storage.clone();
        let model_for_check = model.clone();
        ctx.registrar.tool(
            "check",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let model = model_for_check.clone();
                let storage = storage_for_check.clone();
                Box::pin(async move { check_handler(model, storage, params).await })
            }),
        )?;

        Ok(())
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<PracticePlugin>()
}

async fn generate_handler(model: ModelHandle, params: Value) -> Result<Value, ToolError> {
    let p: GenerateParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let knowledge_point = p.knowledge_point.trim();
    if knowledge_point.is_empty() {
        return Err(ToolError::invalid_params("knowledge_point 不能为空"));
    }
    let difficulty = p.difficulty.unwrap_or_default();
    match build_item(knowledge_point, difficulty) {
        Some(item) => Ok(json!({ "matched": true, "item": item })),
        // P1 智能出题：模板未命中时走 LLM 生成（结构化 schema，见 generate.rs）；
        // 生成失败回退为未命中提示，工具始终可用。
        None => match model_generate(&model, knowledge_point, difficulty).await {
            Ok(item) => Ok(json!({ "matched": true, "source": "llm", "item": item })),
            Err(e) => Ok(json!({
                "matched": false,
                "supported": SUPPORTED_POINTS,
                "message": format!("模板未命中且模型生成失败：{}", e.message),
            })),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::audit::{Auditor, MemoryAuditSink};
    use crate::kernel::plugin::services::{
        AbortSignal, ModelError, ModelHandle, ModelRequest, ModelResponse, ModelService,
        ModelStream,
    };
    use crate::plugin::practice::templates::*;
    use std::sync::{Arc, Mutex};

    struct FakeModel {
        reply: Mutex<String>,
    }

    #[async_trait::async_trait]
    impl ModelService for FakeModel {
        async fn stream(
            &self,
            _request: &ModelRequest,
            _signal: &AbortSignal,
        ) -> Result<ModelStream, ModelError> {
            unreachable!("FakeModel 只服务于 complete")
        }

        async fn complete(
            &self,
            _request: &ModelRequest,
            _signal: &AbortSignal,
        ) -> Result<ModelResponse, ModelError> {
            Ok(ModelResponse {
                text: self.reply.lock().expect("poisoned").clone(),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    fn fake_handle(reply: &str) -> ModelHandle {
        let model: Arc<dyn ModelService> = Arc::new(FakeModel {
            reply: Mutex::new(reply.into()),
        });
        let auditor = Auditor::new(Arc::new(MemoryAuditSink::default()));
        ModelHandle::new(model, std::time::Duration::from_secs(5), auditor)
    }

    #[test]
    fn schema_parses_all_difficulties() {
        for d in ["basic", "variant", "advanced"] {
            let p: GenerateParams = serde_json::from_value(json!({
                "knowledge_point": "绝对值",
                "difficulty": d,
            }))
            .unwrap();
            assert_eq!(
                serde_json::to_value(p.difficulty.unwrap()).unwrap(),
                json!(d)
            );
        }
    }

    #[tokio::test]
    async fn generate_returns_matched_item() {
        let out = generate_handler(
            fake_handle(r#"{"question_text":"不应走到模型生成","answer_spec":""}"#),
            json!({
                "knowledge_point": "三角形全等判定",
                "difficulty": "basic",
            }),
        )
        .await
        .unwrap();
        assert_eq!(out["matched"], true);
        assert_eq!(out["item"]["template_id"], "triangle_sss");
        let spec = out["item"]["diagram_spec"].clone();
        assert!(spec["points"].is_object());
        assert!(spec["objects"].as_array().unwrap().len() >= 6);
    }

    #[tokio::test]
    async fn generate_llm_fallback_returns_item() {
        let out = generate_handler(
            fake_handle(
                r#"{"knowledge_point":"一元二次方程","question_text":"解方程 $x^2-3x+2=0$。","answer_spec":"$x=1$ 或 $x=2$","diagram_spec":null}"#,
            ),
            json!({
                "knowledge_point": "一元二次方程",
                "difficulty": "variant",
            }),
        )
        .await
        .unwrap();
        assert_eq!(out["matched"], true);
        assert_eq!(out["source"], "llm");
        assert_eq!(out["item"]["template_id"], "llm_freeform");
        assert_eq!(out["item"]["difficulty"], "variant");
        assert_eq!(out["item"]["question_text"], "解方程 $x^2-3x+2=0$。");
    }

    #[tokio::test]
    async fn generate_llm_unparseable_falls_back_to_miss() {
        let out = generate_handler(
            fake_handle("抱歉，我无法出题。"),
            json!({
                "knowledge_point": "量子力学",
                "difficulty": "variant",
            }),
        )
        .await
        .unwrap();
        assert_eq!(out["matched"], false);
        assert_eq!(out["supported"].as_array().unwrap().len(), 3);
        assert!(out["message"].as_str().unwrap().contains("模型生成失败"));
    }

    #[test]
    fn three_difficulties_differ_per_template() {
        let qs: Vec<_> = [Difficulty::Basic, Difficulty::Variant, Difficulty::Advanced]
            .iter()
            .map(|d| absolute_value(*d).question_text)
            .collect();
        assert!(qs.windows(2).all(|w| w[0] != w[1]));

        let geo: Vec<_> = [Difficulty::Basic, Difficulty::Variant, Difficulty::Advanced]
            .iter()
            .map(|d| triangle_congruence(*d).template_id.clone())
            .collect();
        assert_eq!(geo.len(), 3);
        assert!(geo[0] != geo[1] && geo[1] != geo[2]);
    }

    #[test]
    fn geometry_diagram_has_labels() {
        let item = triangle_congruence(Difficulty::Advanced);
        let spec = item.diagram_spec.unwrap();
        assert_eq!(spec["labels"].as_array().unwrap().len(), 4);
    }
}
