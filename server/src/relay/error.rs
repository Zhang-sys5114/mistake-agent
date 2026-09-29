//! 中转错误 → HTTP 呈现（ADR-0047 决策 5）。
//!
//! 状态码纪律：
//! - 400 请求非法（非 JSON、非流式）
//! - 402 额度不足（客户端按 402 引导兑换，见 ADR-0048 决策 7）
//! - 429 同一用户并发超限
//! - 502 上游不可用（**上游自己返回的错误码不在这里，走原样透传**）
//! - 500 内部错误（对外只给泛化文案，细节进日志）

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::billing::QuotaDenial;

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("请求体不是合法 JSON")]
    InvalidJson,
    #[error("本网关只接受流式请求（stream: true）")]
    StreamRequired,
    #[error("该路径不支持中转")]
    UnknownPath,
    #[error("同时使用数已达上限（{limit} 个），请等上一个请求结束后重试")]
    TooManyConcurrent { limit: i32 },
    /// 令牌桶限流（按用户或按 IP）
    #[error("请求过于频繁（{scope}），请稍后再试")]
    RateLimited {
        scope: &'static str,
        retry_after_secs: u64,
    },
    /// 全局闸门已满：保护上游账号与进程容量（与"每用户并发"不同，这是全局的）
    #[error("服务当前繁忙，请稍后再试")]
    GlobalBusy { retry_after_secs: u64 },
    #[error("上游服务不可用")]
    Upstream(String),
    #[error("{0}")]
    Quota(QuotaDenial),
    #[error("服务内部错误")]
    Internal(#[from] sqlx::Error),
    #[error("服务内部错误")]
    InternalMsg(String),
}

impl RelayError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            RelayError::InvalidJson => (StatusCode::BAD_REQUEST, "invalid_json"),
            RelayError::StreamRequired => (StatusCode::BAD_REQUEST, "stream_required"),
            RelayError::UnknownPath => (StatusCode::NOT_FOUND, "unknown_path"),
            RelayError::TooManyConcurrent { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "too_many_concurrent")
            }
            RelayError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            RelayError::GlobalBusy { .. } => (StatusCode::TOO_MANY_REQUESTS, "global_busy"),
            RelayError::Upstream(_) => (StatusCode::BAD_GATEWAY, "upstream_unavailable"),
            RelayError::Quota(denial) => (StatusCode::PAYMENT_REQUIRED, denial.code()),
            RelayError::Internal(_) | RelayError::InternalMsg(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        }
    }

    /// 需要在响应里回 `Retry-After` 的错误。
    fn retry_after_secs(&self) -> Option<u64> {
        match self {
            RelayError::RateLimited {
                retry_after_secs, ..
            }
            | RelayError::GlobalBusy { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        }
    }
}

impl IntoResponse for RelayError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        match &self {
            // 额度不足要把具体原因讲清楚，客户端直接展示给用户
            RelayError::Quota(denial) => {
                tracing::info!(reason = denial.code(), "中转请求被额度拒绝");
            }
            RelayError::Upstream(reason) => {
                tracing::warn!(reason = %reason, "上游请求失败");
            }
            RelayError::Internal(_) | RelayError::InternalMsg(_) => {
                tracing::error!(error = ?self, "中转路径内部错误");
            }
            RelayError::RateLimited { scope, .. } => {
                tracing::warn!(scope, "中转请求被限流");
            }
            RelayError::GlobalBusy { .. } => {
                tracing::warn!("全局并发闸门已满，请求被拒");
            }
            _ => {}
        }
        let retry_after = self.retry_after_secs();
        let mut response = (
            status,
            Json(json!({"error": {"code": code, "message": self.to_string()}})),
        )
            .into_response();
        if let Some(secs) = retry_after {
            let value = secs.max(1).to_string();
            if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_matches_client_expectations() {
        assert_eq!(
            RelayError::Quota(QuotaDenial::NoEntitlement).status_and_code(),
            (StatusCode::PAYMENT_REQUIRED, "no_entitlement")
        );
        assert_eq!(
            RelayError::Quota(QuotaDenial::Window5h {
                limit: 15,
                used: 15
            })
            .status_and_code(),
            (StatusCode::PAYMENT_REQUIRED, "window_5h_exceeded")
        );
        assert_eq!(
            RelayError::TooManyConcurrent { limit: 2 }.status_and_code(),
            (StatusCode::TOO_MANY_REQUESTS, "too_many_concurrent")
        );
        assert_eq!(
            RelayError::StreamRequired.status_and_code(),
            (StatusCode::BAD_REQUEST, "stream_required")
        );
        assert_eq!(
            RelayError::Upstream("timeout".into()).status_and_code(),
            (StatusCode::BAD_GATEWAY, "upstream_unavailable")
        );
    }

    #[test]
    fn internal_errors_do_not_leak_details() {
        let err = RelayError::InternalMsg("relation \"x\" does not exist".into());
        assert_eq!(err.to_string(), "服务内部错误");
        assert!(format!("{err:?}").contains("does not exist"));
    }
}
