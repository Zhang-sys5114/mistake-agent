//! session 内核插件：`session::switch` 工具入口（ADR-0030/0035）。
//!
//! 与 Session scheduler（`crate::kernel::agent::session`）分离：调度器是独立内核级
//! 模块（不占 ServiceId），本插件只负责工具声明。实际执行由 agent loop 特殊处理
//! （调用 SessionScheduler::switch），不会走到插件 handler——这里只提供一个兜底错误。

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::KernelContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::registry::{KernelDescriptor, KernelPlugin};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SwitchParams {
    /// 新会话目标（如"批改英语作业"）。
    pub goal: String,
}

pub struct SessionPlugin;

impl KernelPlugin for SessionPlugin {
    fn info() -> Info {
        Info {
            namespace: "session".into(),
            // 会话调度是独立内核级模块（kernel-session），不占 ServiceId（ADR-0030）。
            provides: vec![],
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

    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError> {
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

pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<SessionPlugin>()
}
