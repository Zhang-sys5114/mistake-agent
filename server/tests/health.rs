//! S1 验收：基础设施端点可用，且**不依赖真实数据库**（懒连接池即可构造路由）。
//!
//! 真实数据库的端到端验证（迁移应用、`/readyz` 返回 200）属手工验收步骤，见 `server/README.md`。

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn get(uri: &str) -> (StatusCode, serde_json::Value) {
    let res = common::lazy_app(common::UNREACHABLE_DSN)
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
    let (status, body) = get("/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "mistake-agent-server");
}

#[tokio::test]
async fn readyz_reports_unavailable_when_database_is_unreachable() {
    let (status, body) = get("/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["db"], "error");
}

#[tokio::test]
async fn unknown_path_returns_json_404() {
    let (status, body) = get("/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "未知路径");
}
