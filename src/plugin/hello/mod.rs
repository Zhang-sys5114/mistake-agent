//! demo 插件：hello 工具（hello 回合验收用）。

use serde_json::{Value, json};

use std::sync::Arc;

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, empty_params};
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

pub struct HelloPlugin;

impl UserPlugin for HelloPlugin {
    fn info() -> Info {
        Info {
            namespace: "demo".into(),
            tools: vec![ToolDef {
                name: "hello".into(),
                user_visible: false,
                title: Some("打个招呼".into()),
                group: Some("调试".into()),
                description: "打个招呼，测试 agent 链路是否通畅".into(),
                params: empty_params(),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:hand-wave".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        ctx.registrar.tool(
            "hello",
            Arc::new(|_ctx: &ToolCallContext, _params: Value| {
                Box::pin(async move { Ok(json!({ "reply": "你好，我是错题 Agent" })) })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<HelloPlugin>()
}
