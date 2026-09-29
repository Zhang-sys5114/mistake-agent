//! HTTP 面（ADR-0047 决策 5）。
//!
//! 两条轨道，互不侵入：
//! - `/responses` —— DeepSeek 兼容中转（客户端模型链路，S3 挂载）
//! - `/api/v1/*`  —— 自有业务面：账号 / 套餐 / 同步（S2、S4、S6 挂载）
//!
//! S1 只提供基础设施端点：`/healthz`（存活）、`/readyz`（就绪）与统一 JSON 404。

mod health;

use std::time::Instant;

use axum::Router;
use axum::routing::get;
use sqlx::PgPool;

/// 路由共享状态。刻意保持小：只有数据库池与启动时刻，业务状态各自落在自己的模块里。
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            started_at: Instant::now(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .fallback(health::not_found)
        .with_state(state)
}
