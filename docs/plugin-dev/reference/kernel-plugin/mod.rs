// 内核插件参考模板（复制即开工）。
//
// 用法：
// 1. 把本目录复制到 `src/kernel/plugin/<你的插件名>/`（目录名 = 模块名，小写蛇形）；
// 2. 改 `namespace`，如需提供服务则声明 `provides`（ServiceId）；
// 3. 实现工具/命令/事件；构建即自动收录（ADR-0036），无需改任何聚合文件。
//
// 完整规则见 docs/plugin-dev/kernel.md。
//
// 磁盘 IO 铁律（ADR-0042）：内核插件需要落盘时，构造阶段注入
// `Arc<dyn DomainIo>`（数据根目录域内）或 `Arc<dyn TmpIo>`（系统 temp 暂存），
// 不要裸用 std::fs；路径安全靠 `RelPath::parse`（类型层无遍历），审计内置。

use std::sync::Arc;

use serde_json::{Value, json};

use crate::kernel::context::KernelContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, empty_params};
use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::registry::{KernelDescriptor, KernelPlugin};

pub struct KernelDemoPlugin;

impl KernelPlugin for KernelDemoPlugin {
    fn info() -> Info {
        Info {
            namespace: "kernel_demo".into(),
            // 声明提供的服务（ServiceId::Storage / Memory / Compute / Model）；
            // 每个 ServiceId 至多由一个内核插件提供；session 这类调度模块不占 ServiceId。
            provides: vec![],
            tools: vec![ToolDef {
                name: "ping".into(),
                user_visible: true,
                title: Some("内核示例工具".into()),
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

    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError> {
        // ctx.handles 是全量服务句柄（内核插件在信任边界内，不做 requires 过滤）。
        ctx.registrar.tool(
            "ping",
            Arc::new(|_call_ctx: &ToolCallContext, _params: Value| {
                Box::pin(async move { Ok(json!({ "reply": "pong" })) })
            }),
        )
    }
}

pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<KernelDemoPlugin>()
}
