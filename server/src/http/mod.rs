//! HTTP 面（ADR-0047 决策 5）。
//!
//! 两条轨道，互不侵入：
//! - `/responses` —— DeepSeek 兼容中转（客户端模型链路，S3 挂载）
//! - `/api/v1/*`  —— 自有业务面：账号（S2）、套餐（S4）、同步（S6）
//!
//! 基础设施端点：`/healthz`（存活）、`/readyz`（就绪）与统一 JSON 404。

mod health;

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::routing::get;
use sqlx::PgPool;

use crate::config::Config;

/// 路由共享状态。
///
/// 携带 `Arc<Config>`：鉴权要令牌有效期、中转要平台密钥、计费要权重表——
/// 逐个塞字段会随里程碑膨胀，配置本身是只读的，共享引用最省事。
/// 业务状态（用量、限流窗口）不放这里，各自落在自己的模块。
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(pool: PgPool, config: Arc<Config>) -> Self {
        Self {
            pool,
            config,
            started_at: Instant::now(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .merge(crate::auth::router())
        .merge(crate::admin::router())
        .fallback(health::not_found)
        .with_state(state)
}
