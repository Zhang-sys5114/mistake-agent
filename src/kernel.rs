//! 内核模块入口（Linux 内核组织风格：目录即内核插件）。
//!
//! 内核插件（信任边界内）：`storage/`（会话/错题/审计）、`memory/`（记忆）、
//! `compute/`（验算桥接）、`model/`（双模型适配）；公共契约见 `services.rs`。
//! 用户插件见 `crate::plugin/`（每个插件一个目录，入口 mod.rs 承载插件 info）。

pub mod audit;
pub mod balance;
pub mod cache;
pub mod compute;
pub mod context;
pub mod contract;
pub mod dispatch;
pub mod events;
pub mod logger;
pub mod loop_mod;
pub mod memory;
pub mod message;
pub mod model;
pub mod prompt;
pub mod registry;
pub mod rpc;
pub mod services;
pub mod session;
pub mod settings;
pub mod storage;
