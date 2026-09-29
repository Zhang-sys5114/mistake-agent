//! 不透明访问令牌（ADR-0047 决策 4）。
//!
//! - 形状：`mka_` + 32 字节 CSPRNG 的**小写十六进制**（64 字符）。
//!   选十六进制而非 base64：不需要额外依赖，且不含 `+ / =` 这类在
//!   URL、配置文件、命令行里容易被转义或截断的字符。
//! - 存储：库里只存 SHA-256（`tokens.token_hash`）。库被读走也无法直接冒用，
//!   且正是因为"查库即哈希"，不需要（也不该）用 Argon2 拖慢每个请求。
//! - 明文只在签发那一刻返回一次，服务端不留副本。

use sha2::{Digest, Sha256};
use thiserror::Error;

/// 令牌前缀：便于在日志、工单、误提交的配置里一眼认出这是访问令牌。
pub const PREFIX: &str = "mka_";

const SECRET_BYTES: usize = 32;
const SECRET_HEX_LEN: usize = SECRET_BYTES * 2;

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("随机数生成失败：{0}")]
    Random(String),
}

/// 签发：返回（明文令牌，SHA-256 摘要）。明文交给调用方回给客户端，摘要落库。
pub fn issue() -> Result<(String, Vec<u8>), TokenError> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| TokenError::Random(e.to_string()))?;

    let mut plain = String::with_capacity(PREFIX.len() + SECRET_HEX_LEN);
    plain.push_str(PREFIX);
    for byte in bytes {
        plain.push_str(&format!("{byte:02x}"));
    }
    let hash = hash_of(&plain);
    Ok((plain, hash))
}

/// 令牌 → 查库用的摘要。对任意输入都可算（含垃圾输入），形状校验交给 [`looks_valid`]。
pub fn hash_of(plain: &str) -> Vec<u8> {
    Sha256::digest(plain.as_bytes()).to_vec()
}

/// 形状预检：明显不是我们签发的令牌直接拒绝，不浪费一次数据库查询。
///
/// 只接受**小写**十六进制——签发端只产小写，放宽只会扩大可被灌入的输入空间。
pub fn looks_valid(plain: &str) -> bool {
    let Some(secret) = plain.strip_prefix(PREFIX) else {
        return false;
    };
    secret.len() == SECRET_HEX_LEN
        && secret
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_has_expected_shape_and_is_unique() {
        let (a, hash_a) = issue().unwrap();
        let (b, hash_b) = issue().unwrap();

        assert!(looks_valid(&a), "签发出来的令牌必须自洽：{a}");
        assert_eq!(a.len(), PREFIX.len() + SECRET_HEX_LEN);
        assert_ne!(a, b, "两次签发不能相同");
        assert_ne!(hash_a, hash_b);
        assert_eq!(hash_a.len(), 32, "SHA-256 摘要应为 32 字节");
        assert_eq!(
            hash_of(&a),
            hash_a,
            "摘要必须可按明文复算（查库路径依赖它）"
        );
    }

    #[test]
    fn shape_precheck_rejects_garbage() {
        for bad in [
            "",
            "abc",
            "mka_",
            "mka_short",
            // 大写十六进制：签发端不产生，拒绝以收窄输入空间
            "mka_0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
            // 正确长度但含非十六进制字符
            "mka_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeZ",
            // 少一位
            "mka_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
        ] {
            assert!(!looks_valid(bad), "{bad} 不应通过形状预检");
        }
        assert!(looks_valid(&format!("mka_{}", "0".repeat(SECRET_HEX_LEN))));
    }

    #[test]
    fn hash_is_stable_for_same_input() {
        assert_eq!(hash_of("mka_abc"), hash_of("mka_abc"));
        assert_ne!(hash_of("mka_abc"), hash_of("mka_abd"));
    }
}
