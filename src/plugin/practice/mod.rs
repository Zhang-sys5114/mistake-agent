//! practice 插件：分层变式练习（场景二入口：薄弱点定位 + 分层变式练习）。
//!
//! 插件信息：namespace = practice，requires = [Storage, Model]
//! tools = [generate（变式练习）, gaps（薄弱点定位）]
//! 实现拆分（Linux 内核风格）：`templates.rs` 模板库（题目/答案/图纸同源）；`gaps.rs` 薄弱点聚合

use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::ServiceId;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

mod gaps;
mod templates;

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
        // Model 句柄为 P1「智能出题 / 即时批改」预留：契约已声明，此处先校验存在性。
        ctx.handles
            .model()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Model 句柄".into()))?;

        ctx.registrar.tool(
            "generate",
            std::sync::Arc::new(|_call_ctx: &ToolCallContext, params: Value| {
                Box::pin(async move { generate_handler(params).await })
            }),
        )?;
        ctx.registrar.tool(
            "gaps",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage.clone();
                Box::pin(async move { gaps_handler(storage, params).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<PracticePlugin>()
}

async fn generate_handler(params: Value) -> Result<Value, ToolError> {
    let p: GenerateParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let knowledge_point = p.knowledge_point.trim();
    if knowledge_point.is_empty() {
        return Err(ToolError::invalid_params("knowledge_point 不能为空"));
    }
    match build_item(knowledge_point, p.difficulty.unwrap_or_default()) {
        Some(item) => Ok(json!({ "matched": true, "item": item })),
        None => Ok(json!({
            "matched": false,
            "supported": SUPPORTED_POINTS,
            "message": "当前内置模板仅支持：三角形全等判定、绝对值、一般现在时三单。请用支持的知识点重试。",
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::practice::templates::*;

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
        let out = generate_handler(json!({
            "knowledge_point": "三角形全等判定",
            "difficulty": "basic",
        }))
        .await
        .unwrap();
        assert_eq!(out["matched"], true);
        assert_eq!(out["item"]["template_id"], "triangle_sss");
        let spec = out["item"]["diagram_spec"].clone();
        assert!(spec["points"].is_object());
        assert!(spec["objects"].as_array().unwrap().len() >= 6);
    }

    #[tokio::test]
    async fn generate_returns_supported_list_on_miss() {
        let out = generate_handler(json!({
            "knowledge_point": "量子力学",
            "difficulty": "variant",
        }))
        .await
        .unwrap();
        assert_eq!(out["matched"], false);
        assert_eq!(out["supported"].as_array().unwrap().len(), 3);
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
