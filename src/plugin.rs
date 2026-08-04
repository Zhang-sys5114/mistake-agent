//! 用户插件聚合入口：返回编译期内置的插件描述符列表。

use crate::kernel::registry::PluginDescriptor;

pub mod compute;
pub mod exam;
pub mod grading;
pub mod hello;
pub mod memory;
pub mod practice;
pub mod report;
pub mod session;
pub mod tracking;

/// 编译期内置插件清单（ADR-0002：不做动态加载）。
pub fn builtin_plugins() -> Vec<PluginDescriptor> {
    vec![
        hello::descriptor(),
        grading::descriptor(),
        memory::descriptor(),
        compute::descriptor(),
        practice::descriptor(),
        report::descriptor(),
        exam::descriptor(),
        tracking::descriptor(),
        session::descriptor(),
    ]
}
