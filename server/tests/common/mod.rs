//! 集成测试共享夹具。
//!
//! `tests/` 下每个文件各自编译成一个测试二进制，都会独立包含本模块——
//! 因此"本文件里有、但这个二进制没用到"的夹具会被判为 dead_code。这是共享夹具的固有形态，
//! 统一在文件级放行而不是给每个函数加注解。
#![allow(dead_code)]
//!
//! 两类测试：
//! - **不依赖数据库**（`health.rs`）：懒连接池构造完整路由，任何环境都能跑；
//! - **依赖数据库**（`auth.rs`）：需要可写的 PostgreSQL。连接串取
//!   `TEST_DATABASE_URL` → `DATABASE_URL` → `server/.env`，都没有时给出明确失败信息
//!   （而不是让断言莫名其妙地挂掉）。
//!
//! 隔离策略：每个测试用随机邮箱（[`unique_email`]），互不干扰。
//! 测试库**不清库**——误删开发者手上的数据，代价远大于留下几行测试记录。

use std::sync::Arc;

use axum::Router;
use mistake_agent_server::config::Config;
use mistake_agent_server::http::{AppState, router};
use sqlx::PgPool;
use uuid::Uuid;

/// 只为构造"连不上"的懒连接池用（端口 1 必然不可达）。
pub const UNREACHABLE_DSN: &str = "postgres://u:p@127.0.0.1:1/none";

/// 测试用配置：不读进程环境，避免开发者的 `.env` 影响断言。
pub fn test_config() -> Arc<Config> {
    Arc::new(
        Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some(UNREACHABLE_DSN.to_string()),
            "TOKEN_TTL_DAYS" => Some("1".to_string()),
            _ => None,
        })
        .expect("测试配置构造失败"),
    )
}

/// 带管理员种子的配置：用于验证 `bootstrap_admin` 与管理员鉴权路径。
pub fn config_with_admin(email: &str, password: &str) -> Arc<Config> {
    Arc::new(
        Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some(UNREACHABLE_DSN.to_string()),
            "ADMIN_EMAIL" => Some(email.to_string()),
            "ADMIN_PASSWORD" => Some(password.to_string()),
            _ => None,
        })
        .expect("测试配置构造失败"),
    )
}

/// 懒连接池 + 完整路由：不需要真库的断言用它。
pub fn lazy_app(db_url: &str) -> Router {
    let pool = mistake_agent_server::db::pool_options(1)
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy(db_url)
        .expect("懒连接池构造失败");
    router(AppState::new(pool, test_config()))
}

/// 真实数据库连接池（已应用迁移）。
pub async fn test_pool() -> PgPool {
    let url = database_url();
    let pool = mistake_agent_server::db::connect(&url, 5)
        .await
        .unwrap_or_else(|e| panic!("连接测试数据库失败（{url}）：{e}"));
    mistake_agent_server::db::migrate(&pool)
        .await
        .expect("对测试数据库应用迁移失败");
    pool
}

/// 真库 + 完整路由。
pub fn app(pool: PgPool) -> Router {
    router(AppState::new(pool, test_config()))
}

fn database_url() -> String {
    if let Ok(url) = std::env::var("TEST_DATABASE_URL") {
        return url;
    }
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return url;
    }
    // 本地开发：与手动 `cargo run` 同一份 server/.env
    let _ = dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env"));
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "集成测试需要可写的 PostgreSQL：请设置 TEST_DATABASE_URL 或 DATABASE_URL\
             （或准备好 server/.env，见 server/README.md）"
        )
    })
}

/// 随机邮箱：测试之间互不干扰，也不会撞上开发库里的真实账号。
pub fn unique_email() -> String {
    format!("t-{}@example.test", Uuid::new_v4().simple())
}
