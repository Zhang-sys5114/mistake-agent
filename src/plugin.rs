//! 用户插件聚合入口：返回编译期内置的用户插件描述符列表。
//! 内核插件（storage/memory/compute/model/session）见 `crate::kernel::plugin::builtin_kernel_plugins`（ADR-0035）。

use crate::kernel::registry::PluginDescriptor;

pub mod exam;
pub mod grading;
pub mod hello;
pub mod practice;
pub mod report;
pub mod tracking;

/// 编译期内置插件清单（ADR-0002：不做动态加载）。
pub fn builtin_plugins() -> Vec<PluginDescriptor> {
    vec![
        hello::descriptor(),
        grading::descriptor(),
        practice::descriptor(),
        report::descriptor(),
        exam::descriptor(),
        tracking::descriptor(),
    ]
}
