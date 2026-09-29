//! Mistake Agent 服务端（ADR-0047）。
//!
//! 独立部署单元：与客户端 crate 无 Cargo 关系，不进客户端二进制。
//! 模块按职责划分，落地节奏对应里程碑 S1–S8：
//!
//! - `config`  —— 环境变量配置与校验（S1）
//! - `logging` —— 分级日志（S1）
//! - `db`      —— 连接池与迁移（S1）
//! - `http`    —— 路由装配与基础设施端点（S1），后续挂 `/responses`（中转）与 `/api/v1/*`（业务）
//!
//! 尚未落地（按 ADR-0047/0049）：`auth`（S2）、`relay`（S3）、`billing`（S4）、`sync`（S6）、`admin`（CLI）。

pub mod config;
pub mod db;
pub mod http;
pub mod logging;
