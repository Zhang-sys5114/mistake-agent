//! 入口与插件描述类型（EntryKind/Descriptor/UserPlugin/KernelPlugin）。

//! 注册表（Q11）：启动 fail-fast 校验、两段式契约、懒注册、模型工具列表过滤。

use std::time::Duration;

use schemars::Schema;

use crate::kernel::agent::dispatch::{CommandHandler, EventHandler, ToolHandler};
use crate::kernel::context::{KernelContext, PluginContext};
use crate::kernel::contract::{CallerPolicy, Info, PluginError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Tool,
    Command,
    Event,
}

#[derive(Clone)]
pub enum Handler {
    Tool(ToolHandler),
    Command(CommandHandler),
    Event(EventHandler),
}

#[derive(Clone)]
pub struct RegisteredEntry {
    pub full_name: String,
    pub kind: EntryKind,
    pub policy: CallerPolicy,
    pub timeout: Option<Duration>,
    pub description: String,
    pub icon: Option<String>,
    pub params: Schema,
    pub handler: Handler,
}

/// 编译期内置插件的静态描述符（ADR-0002）。
pub struct PluginDescriptor {
    pub info: Info,
    pub register: fn(PluginContext<'_>) -> Result<(), PluginError>,
}

impl PluginDescriptor {
    pub fn from_plugin<P: UserPlugin>() -> Self {
        Self {
            info: P::info(),
            register: P::register,
        }
    }
}

/// 用户插件两段式契约（ADR-0003）：info 静态元数据，register 绑定 handler。
pub trait UserPlugin {
    fn info() -> Info;
    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError>;
}

/// 编译期内置内核插件描述符（ADR-0035）。
pub struct KernelDescriptor {
    pub info: Info,
    pub register: fn(KernelContext<'_>) -> Result<(), PluginError>,
}

impl KernelDescriptor {
    pub fn from_plugin<P: KernelPlugin>() -> Self {
        Self {
            info: P::info(),
            register: P::register,
        }
    }
}

/// 内核插件两段式契约（ADR-0035）：与 UserPlugin 同形（info + register），
/// 但注册上下文注入**全量**服务句柄——内核插件在信任边界内，是服务的提供者。
pub trait KernelPlugin {
    fn info() -> Info;
    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError>;
}
