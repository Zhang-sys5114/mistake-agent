//! 账号面 HTTP 处理函数与请求/响应 DTO（ADR-0048 决策 3）。
//!
//! 端点是客户端 RPC 的落地形态：
//! `register` / `login` / `logout` / `get_account_status`（→ `GET /api/v1/me`）
//! 与 OOBE 的同步开关（→ `PATCH /api/v1/me`）。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::AuthError;
use super::model::{self, AuthUser, Role, User};
use super::{password, store};
use crate::http::AppState;

// ---------- 注册 ----------

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    /// 展示名：可省（前端回退默认称呼）。
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub user: User,
}

/// 自助注册（ADR-0047 决策 3）。注册即 `user` 角色——`teacher` / `admin` 只能由管理端授予。
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<UserResponse>), AuthError> {
    let email = model::validate_email(&req.email).map_err(AuthError::Validation)?;
    model::validate_password(&req.password).map_err(AuthError::Validation)?;
    let display_name = model::validate_display_name(req.display_name.as_deref().unwrap_or(""))
        .map_err(AuthError::Validation)?;
    let password_hash =
        password::hash(&req.password).map_err(|e| AuthError::InternalMsg(e.to_string()))?;

    let user = store::create_user(
        &state.pool,
        &email,
        &password_hash,
        &display_name,
        Role::User,
    )
    .await?;
    tracing::info!(user_id = %user.id, "新账号注册");
    Ok((StatusCode::CREATED, Json(UserResponse { user })))
}

// ---------- 登录 / 登出 ----------

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// 明文令牌：**只在此刻返回一次**，服务端不留副本（ADR-0047 决策 4）。
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub user: User,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    let email = model::normalize_email(&req.email);
    let Some((user, password_hash)) = store::find_user_for_login(&state.pool, &email).await? else {
        // 账号不存在也走一次等价耗时校验：不让响应时间成为"邮箱是否注册"的探测面
        password::verify_dummy(&req.password);
        return Err(AuthError::InvalidCredentials);
    };
    if !password::verify(&req.password, &password_hash) {
        return Err(AuthError::InvalidCredentials);
    }
    if user.disabled {
        return Err(AuthError::AccountDisabled);
    }

    let (token, expires_at) =
        store::issue_token(&state.pool, user.id, state.config.token_ttl_days).await?;
    tracing::info!(user_id = %user.id, "账号登录");
    Ok(Json(LoginResponse {
        token,
        expires_at,
        user,
    }))
}

/// 登出：只撤销**当前这一个**令牌（其它设备不受影响）。
pub async fn logout(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<StatusCode, AuthError> {
    let revoked = store::revoke_token(&state.pool, auth.token_id).await?;
    tracing::info!(user_id = %auth.user.id, revoked, "账号登出");
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 账号状态 ----------

/// `GET /api/v1/me`：客户端「账户与套餐」卡片的数据源。
/// 套餐与用量字段在 S4 加入（ADR-0048 决策 5），这里先给账号本身。
pub async fn me(auth: AuthUser) -> Json<UserResponse> {
    Json(UserResponse { user: auth.user })
}

#[derive(Debug, Deserialize)]
pub struct UpdateMeRequest {
    /// `Some("")` = 清空展示名；`None` = 不改（与客户端 nickname 语义一致）。
    #[serde(default)]
    pub display_name: Option<String>,
    /// 设备数据同步开关（ADR-0049）：OOBE 询问结果落在这里。
    #[serde(default)]
    pub sync_enabled: Option<bool>,
}

/// `PATCH /api/v1/me`：局部更新（未提供的字段不动）。
pub async fn update_me(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<UpdateMeRequest>,
) -> Result<Json<UserResponse>, AuthError> {
    let display_name = match &req.display_name {
        None => None,
        Some(raw) => Some(model::validate_display_name(raw).map_err(AuthError::Validation)?),
    };

    let user = store::update_profile(
        &state.pool,
        auth.user.id,
        display_name.as_deref(),
        req.sync_enabled,
    )
    .await?;

    if let Some(enabled) = req.sync_enabled {
        tracing::info!(user_id = %user.id, sync_enabled = enabled, "同步开关变更");
    }
    Ok(Json(UserResponse { user }))
}
