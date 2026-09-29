//! DeepSeek 中转（ADR-0047 决策 5，修订 R1–R5）。
//!
//! 定位：**带鉴权与计费的透传网关**。三个协议面（Responses / Chat Completions /
//! Anthropic）在上游都有同名端点，所以这里没有任何协议翻译——只做四件事：
//!
//! 1. 鉴权（复用账号体系的 `AuthUser`）
//! 2. 限额与**预扣**（`billing::reserve`，含并发闸门）
//! 3. 转发 + 流式旁路取 usage（`sse::UsageProbe`，边转发边解析）
//! 4. **结算**（`billing::settle`：按阶梯上调扣次，或失败退回）
//!
//! 服务端**不认识工具调用、不拼上下文、不落正文**——请求与响应正文只在内存里过一遍。

mod concurrency;
mod error;
mod handlers;
mod protocol;
mod sse;
mod upstream;

pub use concurrency::ConcurrencyGate;
pub use error::RelayError;
pub use handlers::relay;
pub use protocol::Protocol;
pub use upstream::build_client;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;

use crate::config::Config;
use crate::http::AppState;

/// 中转路由。
///
/// 同时挂 `/v1/...` 与无前缀两套路径（R1）：OpenAI SDK 与第三方客户端硬编码 `/v1`，
/// 而本项目客户端会把 `api_url` 尾部的 `/v1` 剥掉再拼 `/responses`。
///
/// body 上限单独放大（图片以 base64 内联，可达数 MB），账号面仍保持 axum 的默认上限——
/// 攻击面不跟着放大。
pub fn router(config: &Config) -> Router<AppState> {
    Router::new()
        .route("/responses", post(relay))
        .route("/v1/responses", post(relay))
        .route("/chat/completions", post(relay))
        .route("/v1/chat/completions", post(relay))
        .route("/messages", post(relay))
        .route("/v1/messages", post(relay))
        .layer(DefaultBodyLimit::max(config.relay_max_body_bytes))
}
