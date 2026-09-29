//! 提取器：Bearer 鉴权（[`AuthUser`]）与管理员守卫（[`RequireAdmin`]）。
//!
//! 做成提取器而不是中间件：axum 的提取器把"这个处理函数需要登录"写在**函数签名**上，
//! 漏写就编译不过；中间件方案则是"忘了挂就等于没鉴权"，失败方向相反。

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

use super::error::AuthError;
use super::model::AuthUser;
use super::{store, token};
use crate::http::AppState;

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let plain = bearer_token(parts).ok_or(AuthError::MissingToken)?;
        // 形状预检：明显不是我们签发的令牌不查库
        if !token::looks_valid(plain) {
            return Err(AuthError::InvalidToken);
        }
        let hash = token::hash_of(plain);
        store::authenticate_token(&state.pool, &hash)
            .await?
            .ok_or(AuthError::InvalidToken)
    }
}

/// 管理员守卫：在鉴权之上再要求 `admin` 角色（`teacher` 首期无任何特权）。
pub struct RequireAdmin(pub AuthUser);

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            tracing::warn!(user_id = %user.user.id, role = %user.user.role.as_str(), "非管理员访问管理接口，已拒绝");
            return Err(AuthError::Forbidden);
        }
        Ok(RequireAdmin(user))
    }
}

fn bearer_token(parts: &Parts) -> Option<&str> {
    let raw = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    parse_bearer(raw)
}

/// 解析 `Authorization: Bearer <token>`。方案名大小写不敏感（RFC 7235）。
fn parse_bearer(raw: &str) -> Option<&str> {
    let (scheme, value) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::parse_bearer;

    #[test]
    fn parses_bearer_header_case_insensitively() {
        assert_eq!(parse_bearer("Bearer mka_abc"), Some("mka_abc"));
        assert_eq!(parse_bearer("bearer mka_abc"), Some("mka_abc"));
        assert_eq!(parse_bearer("BEARER   mka_abc  "), Some("mka_abc"));
    }

    #[test]
    fn rejects_other_schemes_and_malformed_headers() {
        for bad in [
            "",
            "mka_abc",
            "Basic dXNlcjpwYXNz",
            "Bearer",
            "Bearer ",
            "Token mka_abc",
        ] {
            assert_eq!(parse_bearer(bad), None, "{bad} 不应被当作 Bearer 令牌");
        }
    }
}
