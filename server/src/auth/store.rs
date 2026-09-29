//! 账号与令牌的数据库读写（ADR-0047 决策 3/4）。
//!
//! 约定：
//! - 行结构体（`*Row`）只在本模块内出现；出模块一律是领域类型（`model::User` / `model::AuthUser`），
//!   口令哈希永不进入领域类型，也就无法被顺手序列化出去。
//! - 唯一约束冲突（邮箱已注册）翻译成语义错误，不把 PG 错误码漏给上层。
//! - 鉴权路径每个请求都读令牌，所以 `last_used_at` 的写入做**节流**，
//!   不让只读路径退化成写热点。

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::error::AuthError;
use super::model::{self, AuthUser, Role, User};
use super::{password, token};

/// `last_used_at` 写入节流：同一令牌 5 分钟内最多更新一次。
const LAST_USED_THROTTLE_MINUTES: i64 = 5;

// ---------- 行结构 ----------

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    role: String,
    display_name: String,
    status: String,
    sync_enabled: bool,
    created_at: DateTime<Utc>,
}

impl UserRow {
    fn into_user(self) -> Result<User, AuthError> {
        let role = Role::parse(&self.role)
            .ok_or_else(|| AuthError::InternalMsg(format!("库中角色非法：{}", self.role)))?;
        Ok(User {
            id: self.id,
            email: self.email,
            role,
            display_name: self.display_name,
            disabled: self.status != "active",
            sync_enabled: self.sync_enabled,
            created_at: self.created_at,
        })
    }
}

/// 登录用：用户行 + 口令哈希（哈希只在本模块内流转）。
#[derive(sqlx::FromRow)]
struct LoginRow {
    id: Uuid,
    email: String,
    role: String,
    display_name: String,
    status: String,
    sync_enabled: bool,
    created_at: DateTime<Utc>,
    password_hash: String,
}

impl LoginRow {
    fn split(self) -> Result<(User, String), AuthError> {
        let password_hash = self.password_hash.clone();
        let user = UserRow {
            id: self.id,
            email: self.email,
            role: self.role,
            display_name: self.display_name,
            status: self.status,
            sync_enabled: self.sync_enabled,
            created_at: self.created_at,
        }
        .into_user()?;
        Ok((user, password_hash))
    }
}

/// 鉴权用：令牌行 + 所属用户行。
#[derive(sqlx::FromRow)]
struct AuthRow {
    token_id: Uuid,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    id: Uuid,
    email: String,
    role: String,
    display_name: String,
    status: String,
    sync_enabled: bool,
    created_at: DateTime<Utc>,
}

// ---------- 用户 ----------

/// 创建账号。邮箱冲突翻译为 [`AuthError::EmailTaken`]。
pub async fn create_user(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    display_name: &str,
    role: Role,
) -> Result<User, AuthError> {
    let row = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (email, password_hash, display_name, role)
         VALUES ($1, $2, $3, $4)
         RETURNING id, email, role, display_name, status, sync_enabled, created_at",
    )
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(role.as_str())
    .fetch_one(pool)
    .await
    .map_err(translate_write_error)?;
    row.into_user()
}

/// 登录用查询：按邮箱取用户与口令哈希。邮箱比较走 `lower()`，
/// 与 `users_email_lower_key` 唯一索引同口径。
pub async fn find_user_for_login(
    pool: &PgPool,
    email: &str,
) -> Result<Option<(User, String)>, AuthError> {
    let row = sqlx::query_as::<_, LoginRow>(
        "SELECT id, email, role, display_name, status, sync_enabled, created_at, password_hash
         FROM users WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    row.map(LoginRow::split).transpose()
}

/// 局部更新（`None` = 不改该字段），返回更新后的用户。字段用 COALESCE 拼接，
/// 避免"读—改—写"两步之间被并发请求插队。
pub async fn update_profile(
    pool: &PgPool,
    user_id: Uuid,
    display_name: Option<&str>,
    sync_enabled: Option<bool>,
) -> Result<User, AuthError> {
    let row = sqlx::query_as::<_, UserRow>(
        "UPDATE users SET
             display_name = COALESCE($2, display_name),
             sync_enabled = COALESCE($3, sync_enabled),
             updated_at   = now()
         WHERE id = $1
         RETURNING id, email, role, display_name, status, sync_enabled, created_at",
    )
    .bind(user_id)
    .bind(display_name)
    .bind(sync_enabled)
    .fetch_optional(pool)
    .await?
    .ok_or(AuthError::InvalidToken)?;
    row.into_user()
}

/// 管理面：分页列出账号（按创建时间倒序）。
pub async fn list_users(pool: &PgPool, limit: i64, offset: i64) -> Result<Vec<User>, AuthError> {
    let rows = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, role, display_name, status, sync_enabled, created_at
         FROM users ORDER BY created_at DESC, id LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(UserRow::into_user).collect()
}

pub async fn find_user_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, AuthError> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, role, display_name, status, sync_enabled, created_at
         FROM users WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    row.map(UserRow::into_user).transpose()
}

// ---------- 令牌 ----------

/// 签发令牌：明文返回给调用方（只此一次），摘要落库。有效期由**数据库时钟**计算，
/// 避免应用与数据库时钟不一致时出现"刚签发就过期"。
pub async fn issue_token(
    pool: &PgPool,
    user_id: Uuid,
    ttl_days: i64,
) -> Result<(String, DateTime<Utc>), AuthError> {
    let (plain, hash) = token::issue().map_err(|e| AuthError::InternalMsg(e.to_string()))?;
    let ttl = format!("{ttl_days} days");
    let expires_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "INSERT INTO tokens (user_id, token_hash, expires_at)
         VALUES ($1, $2, now() + $3::interval)
         RETURNING expires_at",
    )
    .bind(user_id)
    .bind(hash)
    .bind(ttl)
    .fetch_one(pool)
    .await?;
    Ok((plain, expires_at))
}

/// 用摘要换调用方身份。返回 `None` = 令牌不存在/已撤销/已过期；账号停用单独报错。
pub async fn authenticate_token(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<AuthUser>, AuthError> {
    let row = sqlx::query_as::<_, AuthRow>(
        "SELECT t.id AS token_id, t.expires_at, t.revoked_at,
                u.id, u.email, u.role, u.display_name, u.status, u.sync_enabled, u.created_at
         FROM tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };
    if row.revoked_at.is_some() {
        return Ok(None);
    }
    if let Some(expires_at) = row.expires_at
        && expires_at <= Utc::now()
    {
        return Ok(None);
    }
    if row.status != "active" {
        // 令牌本身有效，是账号被停用——区分开才能给出准确的提示
        return Err(AuthError::AccountDisabled);
    }

    let token_id = row.token_id;
    touch_last_used(pool, token_id).await;

    let user = UserRow {
        id: row.id,
        email: row.email,
        role: row.role,
        display_name: row.display_name,
        status: row.status,
        sync_enabled: row.sync_enabled,
        created_at: row.created_at,
    }
    .into_user()?;
    Ok(Some(AuthUser { user, token_id }))
}

/// 撤销令牌（登出）。返回是否真的撤销了一个有效令牌。
pub async fn revoke_token(pool: &PgPool, token_id: Uuid) -> Result<bool, AuthError> {
    let result =
        sqlx::query("UPDATE tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(token_id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// 节流更新 `last_used_at`。失败只记警告——审计用途的写入不该让鉴权失败。
async fn touch_last_used(pool: &PgPool, token_id: Uuid) {
    let cutoff = Utc::now() - Duration::minutes(LAST_USED_THROTTLE_MINUTES);
    let result = sqlx::query(
        "UPDATE tokens SET last_used_at = now()
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2)",
    )
    .bind(token_id)
    .bind(cutoff)
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, "更新令牌 last_used_at 失败");
    }
}

// ---------- 管理员种子 ----------

/// 创建/核对管理员种子（ADR-0047 决策 3）。返回是否真的创建了。
///
/// **按邮箱幂等**，而不是"库里有管理员就整体跳过"：
/// - 该邮箱不存在 → 建为 admin；
/// - 该邮箱已是 admin → 跳过（重复启动无副作用）；
/// - 该邮箱被普通账号占用 → 只记 WARNING 并跳过。**刻意不提升同邮箱账号**——
///   否则"抢先注册种子邮箱"就成了一条提权路径，提权留给 CLI（S4）。
pub async fn ensure_seed_admin(
    pool: &PgPool,
    email: &str,
    password: &str,
) -> Result<bool, AuthError> {
    let email = model::validate_email(email).map_err(AuthError::Validation)?;
    model::validate_password(password).map_err(AuthError::Validation)?;

    if let Some(existing) = find_user_by_email(pool, &email).await? {
        if existing.role != Role::Admin {
            tracing::warn!(
                email = %email,
                "管理员种子邮箱已被普通账号占用，跳过创建（如需提权请用 admin CLI）"
            );
        }
        return Ok(false);
    }

    let password_hash =
        password::hash(password).map_err(|e| AuthError::InternalMsg(e.to_string()))?;
    let user = create_user(pool, &email, &password_hash, "管理员", Role::Admin).await?;
    tracing::info!(email = %user.email, "已创建管理员种子账号");
    Ok(true)
}

/// 唯一约束冲突 → 语义错误；其余原样上抛。
fn translate_write_error(error: sqlx::Error) -> AuthError {
    if let sqlx::Error::Database(db) = &error
        && db.code().as_deref() == Some("23505")
    {
        return AuthError::EmailTaken;
    }
    AuthError::Internal(error)
}
