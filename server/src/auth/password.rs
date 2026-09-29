//! 口令哈希（ADR-0047 决策 4）：Argon2id，每个口令独立随机盐。
//!
//! 盐由 `getrandom` 自行生成后经 `SaltString::encode_b64` 编码，而不是用 password-hash
//! 重导出的 `OsRng`——少一条对上游重导出的隐式依赖，随机源也只有一个出处。

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use thiserror::Error;

/// 盐长度（字节）：password-hash 要求 ≥ 8，16 是通行取值。
const SALT_BYTES: usize = 16;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("随机盐生成失败：{0}")]
    Random(String),
    #[error("口令哈希生成失败")]
    Hash,
}

/// 生成 PHC 字符串（含算法参数与盐），可直接落库。
pub fn hash(password: &str) -> Result<String, PasswordError> {
    let mut salt = [0u8; SALT_BYTES];
    getrandom::fill(&mut salt).map_err(|e| PasswordError::Random(e.to_string()))?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| PasswordError::Hash)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordError::Hash)
}

/// 校验口令。哈希串不可解析（历史脏数据）时返回 false——宁可让用户重设，也不放行。
pub fn verify(password: &str, encoded: &str) -> bool {
    match PasswordHash::new(encoded) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// 账号不存在时也走一次等价耗时的校验，避免用响应时间枚举"哪些邮箱已注册"。
///
/// 对照哈希在首次调用时懒生成并常驻内存（不从库读、不经配置），
/// 因此它的存在不构成任何可被利用的凭据。
pub fn verify_dummy(password: &str) {
    static DUMMY: OnceLock<Option<String>> = OnceLock::new();
    let encoded = DUMMY.get_or_init(|| hash("mistake-agent-timing-equalizer").ok());
    if let Some(encoded) = encoded {
        let _ = verify(password, encoded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let encoded = hash("correct horse battery").unwrap();
        assert!(
            encoded.starts_with("$argon2id$"),
            "应为 Argon2id PHC 串：{encoded}"
        );
        assert!(verify("correct horse battery", &encoded));
        assert!(!verify("wrong horse battery", &encoded));
    }

    #[test]
    fn same_password_hashes_differently_per_call() {
        // 每个口令独立随机盐：两次哈希不同，但都能校验通过
        let a = hash("same-password").unwrap();
        let b = hash("same-password").unwrap();
        assert_ne!(a, b);
        assert!(verify("same-password", &a));
        assert!(verify("same-password", &b));
    }

    #[test]
    fn malformed_hash_never_verifies() {
        for bad in ["", "not-a-phc-string", "$argon2id$broken"] {
            assert!(!verify("whatever", bad), "{bad} 不应通过校验");
        }
    }

    #[test]
    fn dummy_verify_does_not_panic() {
        verify_dummy("anything");
        verify_dummy("");
    }
}
