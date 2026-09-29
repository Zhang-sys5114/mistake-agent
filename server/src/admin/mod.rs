//! 管理面（ADR-0047 决策 8）：首期以 REST + CLI 提供，不做网页管理台。
//!
//! S2 落地的第一个管理端点是账号列表——它同时是**角色守卫的第一个真实用户**：
//! 没有受保护端点，"越权被拒绝"就只有单测没有落点。

mod handlers;

use axum::Router;
use axum::routing::get;

use crate::http::AppState;

/// `/api/v1/admin` 下的管理面。所有路由都要求 `admin` 角色（在 handler 签名里由守卫强制）。
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/admin/users", get(handlers::list_users))
}
