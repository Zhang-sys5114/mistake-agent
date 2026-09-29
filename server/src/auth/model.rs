//! 账号领域类型与输入校验。
//!
//! 字段校验刻意"最低限度"：邮箱只判形状（完整 RFC 5322 校验收益低、误拒真实邮箱的成本高），
//! 口令只判长度（强度交给用户与后续的弱口令名单，不做复杂度硬性要求）。
//! 真相源是数据库的 CHECK 约束（`role` / `status`），这里的 `parse` 与之一一对应。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 口令长度下限：低于此值连暴力破解都不需要。
pub const MIN_PASSWORD_CHARS: usize = 8;
/// 口令长度上限：挡住把长文本当口令（Argon2 成本随长度线性上升）。
pub const MAX_PASSWORD_CHARS: usize = 128;
/// 展示名长度上限：与客户端昵称一致（`settings.json` 的 nickname ≤ 24 字符）。
pub const MAX_DISPLAY_NAME_CHARS: usize = 24;

/// 账号角色（ADR-0047 决策 3）。`Teacher` 首期占位：建角色、无功能。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Teacher,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Teacher => "teacher",
            Role::Admin => "admin",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "user" => Some(Role::User),
            "teacher" => Some(Role::Teacher),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// 账号（**不含口令哈希**：该字段只在 `store` 内部流转，永不出现在响应里）。
#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub display_name: String,
    /// 停用标记（`status != 'active'`）。
    pub disabled: bool,
    /// 设备数据同步开关（ADR-0049）：默认关闭。
    pub sync_enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// 已鉴权的调用方：由 `AuthUser` 提取器注入处理函数。
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user: User,
    /// 本次请求携带的令牌 id（登出时据此撤销）。
    pub token_id: Uuid,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.user.role == Role::Admin
    }
}

/// 邮箱归一化：去空白 + 转小写。
///
/// 与库里的 `users_email_lower_key`（按 `lower(email)` 唯一）保持一致——
/// 归一化只发生在这里一处，避免"注册时大写、登录时小写"查出两个账号。
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// 邮箱形状校验，返回归一化后的值。
pub fn validate_email(raw: &str) -> Result<String, &'static str> {
    let email = normalize_email(raw);
    if email.chars().count() > 254 {
        return Err("邮箱过长");
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err("邮箱格式不正确");
    };
    if local.is_empty() || domain.is_empty() {
        return Err("邮箱格式不正确");
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err("邮箱格式不正确");
    }
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("邮箱格式不正确");
    }
    Ok(email)
}

pub fn validate_password(raw: &str) -> Result<(), &'static str> {
    let len = raw.chars().count();
    if len < MIN_PASSWORD_CHARS {
        return Err("口令至少 8 个字符");
    }
    if len > MAX_PASSWORD_CHARS {
        return Err("口令过长");
    }
    Ok(())
}

/// 展示名：去空白；空串合法（前端回退默认称呼）；超长拒绝。
pub fn validate_display_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err("昵称过长");
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_normalized_and_shape_checked() {
        assert_eq!(validate_email("  A@B.COM ").unwrap(), "a@b.com");
        for bad in [
            "",
            "no-at-sign",
            "@example.com",
            "user@",
            "user@localhost",
            "user@.com",
            "user@com.",
            "a b@example.com",
            "two@at@example.com",
        ] {
            assert!(validate_email(bad).is_err(), "{bad} 应被拒绝");
        }
        assert!(validate_email(&format!("{}@example.com", "a".repeat(250))).is_err());
    }

    #[test]
    fn password_length_bounds() {
        assert!(validate_password("1234567").is_err());
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password(&"x".repeat(MAX_PASSWORD_CHARS)).is_ok());
        assert!(validate_password(&"x".repeat(MAX_PASSWORD_CHARS + 1)).is_err());
    }

    #[test]
    fn display_name_is_trimmed_and_bounded() {
        assert_eq!(validate_display_name("  张涵 ").unwrap(), "张涵");
        assert_eq!(validate_display_name("   ").unwrap(), "");
        // 上限按**字符**算：24 个汉字放行，25 个拒绝（与客户端 nickname 一致）
        assert!(validate_display_name(&"字".repeat(MAX_DISPLAY_NAME_CHARS)).is_ok());
        assert!(validate_display_name(&"字".repeat(MAX_DISPLAY_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn role_roundtrip_matches_database_check_constraint() {
        for role in [Role::User, Role::Teacher, Role::Admin] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("root"), None);
    }
}
