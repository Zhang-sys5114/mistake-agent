//! HTTP 面（ADR-0047 决策 5）。
//!
//! 两条轨道，互不侵入：
//! - `/responses`、`/chat/completions`、`/v1/messages` 等 —— DeepSeek 兼容中转（`relay`）
//! - `/api/v1/*` —— 自有业务面：账号（S2）、套餐（S4）、同步（S6）
//!
//! 基础设施端点：`/healthz`（存活）、`/readyz`（就绪）与统一 JSON 404。

mod health;

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::routing::get;
use sqlx::PgPool;

use crate::config::Config;
use crate::relay::ConcurrencyGate;
use crate::security::SecurityState;

/// 路由共享状态。
///
/// 携带 `Arc<Config>`：鉴权要令牌有效期、中转要平台密钥与限流参数、计费要阶梯阈值——
/// 逐个塞字段会随里程碑膨胀，配置是只读的，共享引用最省事。
/// 业务状态（用量、限流窗口）不放这里，各自落在自己的模块。
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub started_at: Instant,
    /// 上游客户端（面向长流式响应，刻意不设总超时）
    pub upstream: reqwest::Client,
    /// 每用户并发闸门（进程内；水平扩展时须挪到共享存储，见 ADR-0047 修订 R9）
    pub relay_gate: Arc<ConcurrencyGate>,
    /// 安全护栏：限流、失败封禁、全局并发上限
    pub security: Arc<SecurityState>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Arc<Config>) -> Self {
        let security = Arc::new(SecurityState::new(config.security.clone()));
        Self {
            pool,
            upstream: crate::relay::build_client(),
            relay_gate: Arc::new(ConcurrencyGate::new()),
            security,
            config,
            started_at: Instant::now(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let relay = crate::relay::router(&state.config);
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .merge(crate::auth::router())
        .merge(crate::admin::router())
        .merge(relay)
        .fallback(health::not_found)
        .with_state(state)
}
