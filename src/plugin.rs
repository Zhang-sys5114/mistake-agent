//! 用户插件聚合入口：返回编译期内置的插件描述符列表。

use crate::kernel::registry::PluginDescriptor;

pub mod grading;
pub mod hello;

/// 编译期内置插件清单（ADR-0002：不做动态加载）。
pub fn builtin_plugins() -> Vec<PluginDescriptor> {
    vec![hello::descriptor(), grading::descriptor()]
}
