//! 鉴权错误与其 HTTP 呈现（ADR-0047 决策 5）。
//!
//! 错误体统一 `{"error":{"code":"...","message":"..."}}`：客户端按**状态码**分流
//! （401 → 引导重新登录、403 → 权限或停用、409 → 邮箱已注册），`code` 供更细的分支。
//!
//! 纪律：内部错误（数据库等）对外只回一句泛化文案，细节进日志——
//! 库结构、SQL 状态码不该出现在响应体里。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("邮箱或口令不正确")]
    InvalidCredentials,
    #[error("该邮箱已注册")]
    EmailTaken,
    #[error("未提供访问令牌")]
    MissingToken,
    #[error("访问令牌无效或已失效")]
    InvalidToken,
    #[error("账号已被停用")]
    AccountDisabled,
    #[error("无权访问该资源")]
    Forbidden,
    #[error("{0}")]
    Validation(&'static str),
    /// 令牌桶限流（账号面按 IP）
    #[error("请求过于频繁，请稍后再试")]
    RateLimited { retry_after_secs: u64 },
    /// 失败封禁（fail2ban 语义）：反复登录失败后临时禁用
    #[error("登录失败次数过多，已临时禁用，请稍后再试")]
    LoginBlocked { retry_after_secs: u64 },
    #[error("服务内部错误")]
    Internal(#[from] sqlx::Error),
    #[error("服务内部错误")]
    InternalMsg(String),
}

impl AuthError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            AuthError::EmailTaken => (StatusCode::CONFLICT, "email_taken"),
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "missing_token"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "invalid_token"),
            AuthError::AccountDisabled => (StatusCode::FORBIDDEN, "account_disabled"),
            AuthError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            AuthError::Validation(_) => (StatusCode::BAD_REQUEST, "validation_failed"),
            AuthError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            AuthError::LoginBlocked { .. } => (StatusCode::TOO_MANY_REQUESTS, "login_blocked"),
            AuthError::Internal(_) | AuthError::InternalMsg(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        }
    }

    /// 需要在响应里回 `Retry-After` 的错误（客户端据此退避，比盲目重试友好）。
    fn retry_after_secs(&self) -> Option<u64> {
        match self {
            AuthError::RateLimited { retry_after_secs }
            | AuthError::LoginBlocked { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            // Debug 而非 Display：thiserror 的 Display 是泛化文案，源错误只在 Debug 里
            tracing::error!(error = ?self, "鉴权路径内部错误");
        }
        let retry_after = self.retry_after_secs();
        let mut response = (
            status,
            Json(json!({"error": {"code": code, "message": self.to_string()}})),
        )
            .into_response();
        if let Some(secs) = retry_after {
            // 秒数取整到至少 1：Retry-After: 0 会鼓励客户端立刻重试
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

    fn status_of(err: AuthError) -> (StatusCode, &'static str) {
        err.status_and_code()
    }

    #[test]
    fn status_mapping_matches_client_expectations() {
        // 客户端按状态码分流（ADR-0048 决策 7）：401 重新登录、403 停用/越权、409 已注册
        assert_eq!(
            status_of(AuthError::InvalidCredentials),
            (StatusCode::UNAUTHORIZED, "invalid_credentials")
        );
        assert_eq!(
            status_of(AuthError::InvalidToken),
            (StatusCode::UNAUTHORIZED, "invalid_token")
        );
        assert_eq!(
            status_of(AuthError::AccountDisabled),
            (StatusCode::FORBIDDEN, "account_disabled")
        );
        assert_eq!(
            status_of(AuthError::Forbidden),
            (StatusCode::FORBIDDEN, "forbidden")
        );
        assert_eq!(
            status_of(AuthError::EmailTaken),
            (StatusCode::CONFLICT, "email_taken")
        );
        assert_eq!(
            status_of(AuthError::Validation("口令至少 8 个字符")),
            (StatusCode::BAD_REQUEST, "validation_failed")
        );
        assert_eq!(
            status_of(AuthError::InternalMsg("boom".into())),
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        );
    }

    #[test]
    fn internal_errors_do_not_leak_details_in_message() {
        let err = AuthError::InternalMsg("relation \"secrets\" does not exist".into());
        // 对外文案是泛化的；细节只经 Debug 进日志
        assert_eq!(err.to_string(), "服务内部错误");
        assert!(format!("{err:?}").contains("secrets"));
    }
}
