//! 内核插件层（kernel/plugin）：信任边界内的特权子系统，一个插件一个文件夹，
//! 入口 mod.rs 承载插件 info（与用户插件组织方式一致，Linux 内核组织风格）。
//!
//! - `services`：内核插件公共契约（ServiceId / 服务 trait / 受控句柄）；
//! - 各内核插件目录经 `KernelPlugin` 两段式契约注册（ADR-0035），与用户插件同表校验；
//! - 插件清单由 build.rs 自动发现（ADR-0036），新增插件无需改本文件。
//!   规则见 docs/plugin-dev/kernel.md；参考模板见 docs/plugin-dev/reference/kernel-plugin/。

pub mod services;

include!(concat!(env!("OUT_DIR"), "/builtin_kernel_plugins.rs"));

#[cfg(test)]
mod tests {
    // 编译锚定：参考模板必须始终与真实契约一致（不注册，仅编译检查）。
    include!("../../../docs/plugin-dev/reference/kernel-plugin/mod.rs");

    #[test]
    fn kernel_plugin_reference_typechecks() {
        let _ = descriptor();
    }
}
