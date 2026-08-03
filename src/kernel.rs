//! 内核模块入口（Rust 2018 布局，无 mod.rs）。

pub mod audit;
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
