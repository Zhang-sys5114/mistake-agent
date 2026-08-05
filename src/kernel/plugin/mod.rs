//! 内核插件层（kernel/plugin）：信任边界内的特权子系统，一个插件一个文件夹，
//! 入口 mod.rs 承载插件 info（与用户插件组织方式一致，Linux 内核组织风格）。
//!
//! - `services`：内核插件公共契约（ServiceId / 服务 trait / 受控句柄）；
//! - `storage` / `memory` / `compute` / `model` / `session`：各内核插件，
//!   经 `KernelPlugin` 两段式契约注册（ADR-0035），与用户插件同表校验。

pub mod compute;
pub mod memory;
pub mod model;
pub mod services;
pub mod session;
pub mod storage;

use crate::kernel::registry::KernelDescriptor;

/// 编译期内置内核插件清单（ADR-0035：与用户插件同机制注册、同一张表校验）。
pub fn builtin_kernel_plugins() -> Vec<KernelDescriptor> {
    vec![
        storage::descriptor(),
        memory::descriptor(),
        compute::descriptor(),
        model::descriptor(),
        session::descriptor(),
    ]
}
