//! S1 验收：基础设施端点可用，且**不依赖真实数据库**（懒连接池即可构造路由）。
//!
//! 真实数据库的端到端验证（迁移应用、`/readyz` 返回 200）属手工验收步骤，见 `server/README.md`。

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mistake_agent_server::http::{AppState, router};
use tower::ServiceExt;

/// 懒连接池：不实际建连，所以 `/healthz`、404 可离线断言；`/readyz` 用于断言"库不可达"分支。
///
/// 连接超时压到 300ms：不可达时 sqlx 会一直重试到超时，用生产值（5s）只会让测试白等。
fn app(db_url: &str) -> Router {
    let pool = mistake_agent_server::db::pool_options(1)
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy(db_url)
        .expect("懒连接池构造失败");
    router(AppState::new(pool))
}

async fn get(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let res = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("请求构造失败"),
        )
        .await
        .expect("路由调用失败");
    let status = res.status();
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("读取响应体失败")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

#[tokio::test]
async fn healthz_is_ok_without_database() {
    let (status, body) = get(app("postgres://u:p@127.0.0.1:1/none"), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "mistake-agent-server");
}

#[tokio::test]
async fn readyz_reports_unavailable_when_database_is_unreachable() {
    let (status, body) = get(app("postgres://u:p@127.0.0.1:1/none"), "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["db"], "error");
}

#[tokio::test]
async fn unknown_path_returns_json_404() {
    let (status, body) = get(app("postgres://u:p@127.0.0.1:1/none"), "/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "未知路径");
}
