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
//! - `http`    —— 路由装配与基础设施端点（S1），`/api/v1/*` 由各业务模块挂载
//!
//! 尚未落地（按 ADR-0047/0049）：`relay`（S3 中转）、`billing`（S4 套餐与兑换码）、`sync`（S6）。

pub mod admin;
pub mod auth;
pub mod config;
pub mod db;
pub mod http;
pub mod logging;
