//! session 插件：回合内主动切换会话（ADR-0030）。
//! 工具声明走标准插件通道（模型可见、懒加载前 Eager），
//! 实际执行由 agent loop 特殊处理（调用 SessionScheduler::switch），
//! 不会走到插件 handler——这里只提供一个兜底错误。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::dispatch::ToolCallContext;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SwitchParams {
    /// 新会话目标（如"批改英语作业"）。
    pub goal: String,
}

pub struct SessionPlugin;

impl UserPlugin for SessionPlugin {
    fn info() -> Info {
        Info {
            namespace: "session".into(),
            load: crate::kernel::contract::LoadPolicy::Eager,
            tools: vec![ToolDef {
                name: "switch".into(),
                user_visible: false,
                title: Some("切换会话".into()),
                group: None,
                description:
                    "结束当前会话并开启一个围绕新目标的新会话（自动携带近期历史）。用法：session::switch <新目标>"
                        .into(),
                params: schemars::schema_for!(SwitchParams),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:swap-horizontal".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        ctx.registrar.tool(
            "switch",
            Arc::new(|_ctx: &ToolCallContext, _p: Value| {
                Box::pin(async move {
                    Err(ToolError::handler(
                        "会话切换由 agent loop 特殊处理，不能直接调用",
                    ))
                })
            }),
        )?;
        Ok(())
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<SessionPlugin>()
}
