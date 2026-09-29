//! 基础设施端点：存活、就绪、统一 404。
//!
//! 分工刻意明确：`/healthz` **不碰数据库**（进程活着就返回 200，供 systemd/容器判活），
//! `/readyz` 才探数据库（供反向代理摘流）。把两者混在一起会导致数据库抖动时进程被反复重启。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};

use super::AppState;

pub async fn healthz() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "mistake-agent-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = crate::db::ping(&state.pool).await;
    let body = json!({
        "status": if db_ok { "ready" } else { "unavailable" },
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "db": if db_ok { "ok" } else { "error" },
    });
    let code = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
}

/// 未知路径统一 JSON 错误体（`{"error":{"message":...}}`）——
/// 与 ADR-0047 决策 5 的错误体形状一致，客户端可统一解析。
pub async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": {"message": "未知路径"}})),
    )
}
