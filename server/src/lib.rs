//! Mistake Agent 服务端（ADR-0047）。
//!
//! 独立部署单元：与客户端 crate 无 Cargo 关系，不进客户端二进制。
//! 模块按职责划分，落地节奏对应里程碑 S1–S8：
//!
//! - `config`  —— 环境变量配置与校验（S1）
//! - `logging` —— 分级日志 + 脱敏（S1）
//! - `db`      —— 连接池与迁移（S1）
//! - `auth`    —— 账号、令牌与鉴权（S2）
//! - `admin`   —— 管理面：账号查询与（后续）兑换码发放（S2 起）
//! - `billing` —— 套餐、权益、用量与限额裁决（S3 起）
//! - `relay`   —— DeepSeek 三协议透传网关（S3 起）
//! - `security`—— 限流、失败封禁与全局并发上限（S3 起）
//! - `http`    —— 路由装配与基础设施端点（S1），业务由各模块挂载
//!
//! 尚未落地（按 ADR-0047/0049）：`sync`（S6 同步）。
//! 里程碑边界在 S3 做了调整：`plans`/`entitlements`/`usage_events` 从 S4 提前到 S3
//! （中转没有权益就无从限流），S4 保留兑换码、admin CLI 与套餐数值校准。

pub mod admin;
pub mod auth;
pub mod billing;
pub mod config;
pub mod db;
pub mod http;
pub mod logging;
pub mod relay;
pub mod security;
