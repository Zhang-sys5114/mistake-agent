// 用户插件参考模板（复制即开工）。
//
// 用法：
// 1. 把本目录复制到 `src/plugin/<你的插件名>/`（目录名 = 模块名，小写蛇形）；
// 2. 改 `namespace` 为你的命名空间（全局唯一，工具全名 = `命名空间::工具名`）；
// 3. 实现你要的工具/命令/事件；构建即自动收录（ADR-0036），无需改任何聚合文件。
//
// 完整规则见 docs/plugin-dev/user.md。

use std::sync::Arc;

use serde_json::{Value, json};

use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, empty_params};
use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

pub struct UserDemoPlugin;

impl UserPlugin for UserDemoPlugin {
    fn info() -> Info {
        Info {
            namespace: "user_demo".into(),
            // 声明需要的服务句柄（ServiceId::Storage / Model / Memory / Compute）。
            requires: vec![],
            tools: vec![ToolDef {
                name: "ping".into(),
                user_visible: true,
                title: Some("示例工具".into()),
                group: Some("示例".into()),
                description: "示例工具：返回 pong。复制后改成你的工具。".into(),
                params: empty_params(),
                policy: CallerPolicy::UserAndModel,
                timeout: None,
                icon: Some("mdi:hand-wave".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        // ctx.handles 只包含 requires 声明的服务（受控句柄，如 StorageHandle/ModelHandle）。
        // ctx.registrar 只允许登记 info 里声明过的短名（声明与实现一致）。
        ctx.registrar.tool(
            "ping",
            Arc::new(|_call_ctx: &ToolCallContext, _params: Value| {
                Box::pin(async move { Ok(json!({ "reply": "pong" })) })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<UserDemoPlugin>()
}
