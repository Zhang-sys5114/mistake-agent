//! 服务端配置（ADR-0047 决策 2）：**环境变量是唯一来源**，启动时一次性读取并校验（fail-fast）。
//!
//! 生产经 systemd `EnvironmentFile` 注入；开发期由 `load_dotenv()` 读 `server/.env`（缺失不算错误）。
//! 解析逻辑走 `from_lookup`，把"取值来源"与"解析校验"分开，使校验规则可测（不依赖进程环境变量）。

use std::env;
use std::net::SocketAddr;

use thiserror::Error;

use crate::security::{
    DEFAULT_AUTH_BURST, DEFAULT_AUTH_RATE_PER_MINUTE, DEFAULT_LOGIN_BLOCK_SECS,
    DEFAULT_LOGIN_FAILURE_WINDOW_SECS, DEFAULT_LOGIN_MAX_FAILURES, DEFAULT_MAX_CONCURRENT_GLOBAL,
    DEFAULT_RELAY_BURST, DEFAULT_RELAY_RATE_PER_MINUTE, DEFAULT_TRUST_PROXY, SecuritySettings,
};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("缺少必需的环境变量 {0}")]
    Missing(&'static str),
    #[error("环境变量 {name} 取值非法：{reason}")]
    Invalid { name: &'static str, reason: String },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub db_max_connections: u32,
    pub log_level: String,
    /// 平台 DeepSeek 密钥：S3 中转使用；S1 允许为空（启动告警）。
    pub deepseek_api_key: String,
    pub deepseek_base_url: String,
    /// 中转时强制使用的上游模型名（不信任客户端传来的 model）。
    pub deepseek_model: String,
    /// 阶梯扣次的 token 阈值（升序）：≤ 第一个阈值扣 1 次，每跨过一个阈值多扣 1 次。
    /// 用途是防止单次超长上下文击穿"按次数售卖"（ADR-0047 决策 6）。
    pub billing_ladder_tokens: Vec<u64>,
    /// 中转请求体上限（字节）：图片以 base64 内联，可达数 MB。
    pub relay_max_body_bytes: usize,
    /// 同一用户同时在飞的中转请求上限（ADR-0047 决策 9）。
    pub relay_max_concurrent_per_user: i32,
    /// 登录令牌有效期（天）：桌面端长期在线，默认 90 天。
    pub token_ttl_days: i64,
    /// 首个管理员种子（ADR-0047 决策 3）：两者都配齐且库中尚无管理员时创建，幂等。
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    /// 安全护栏（限流/失败封禁/全局并发）：一组配置，避免 `Config` 顶层字段继续膨胀。
    pub security: SecuritySettings,
}

/// 默认监听回环：TLS 由前置反向代理终结，服务端不直接对外（ADR-0047 决策 2）。
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_DEEPSEEK_MODEL: &str = "deepseek-flash";
const DEFAULT_TOKEN_TTL_DAYS: i64 = 90;
/// 32k / 64k：与 ADR-0047 决策 6 的初始档位一致，数值随真实用量数据校准。
const DEFAULT_BILLING_LADDER_TOKENS: [u64; 2] = [32 * 1024, 64 * 1024];
/// 32 MiB：够放下几张手机原图（base64 后膨胀 ~33%），又不至于让单请求吃爆内存。
const DEFAULT_RELAY_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_RELAY_MAX_CONCURRENT_PER_USER: i32 = 2;

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// 从任意取值来源构造配置（测试与 admin CLI 复用）。
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let bind_raw = lookup("BIND_ADDR").unwrap_or_else(|| DEFAULT_BIND_ADDR.to_string());
        let bind_addr = bind_raw
            .parse::<SocketAddr>()
            .map_err(|e| ConfigError::Invalid {
                name: "BIND_ADDR",
                reason: e.to_string(),
            })?;

        let database_url = lookup("DATABASE_URL")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or(ConfigError::Missing("DATABASE_URL"))?;

        let db_max_connections = match lookup("DB_MAX_CONNECTIONS") {
            None => DEFAULT_DB_MAX_CONNECTIONS,
            Some(raw) => raw
                .trim()
                .parse::<u32>()
                .map_err(|e| ConfigError::Invalid {
                    name: "DB_MAX_CONNECTIONS",
                    reason: e.to_string(),
                })?,
        };
        if db_max_connections == 0 {
            return Err(ConfigError::Invalid {
                name: "DB_MAX_CONNECTIONS",
                reason: "必须大于 0".into(),
            });
        }

        let log_level = lookup("LOG_LEVEL")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string());

        let token_ttl_days = match lookup("TOKEN_TTL_DAYS") {
            None => DEFAULT_TOKEN_TTL_DAYS,
            Some(raw) => raw
                .trim()
                .parse::<i64>()
                .map_err(|e| ConfigError::Invalid {
                    name: "TOKEN_TTL_DAYS",
                    reason: e.to_string(),
                })?,
        };
        if token_ttl_days <= 0 {
            return Err(ConfigError::Invalid {
                name: "TOKEN_TTL_DAYS",
                reason: "必须大于 0".into(),
            });
        }

        // 管理员种子：两个都给了才生效，只给一个视为配置漏项（不静默忽略）
        let admin_email = lookup("ADMIN_EMAIL")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let admin_password = lookup("ADMIN_PASSWORD").filter(|v| !v.is_empty());
        if admin_email.is_some() != admin_password.is_some() {
            return Err(ConfigError::Invalid {
                name: "ADMIN_EMAIL",
                reason: "ADMIN_EMAIL 与 ADMIN_PASSWORD 必须同时提供".into(),
            });
        }

        let billing_ladder_tokens = match lookup("BILLING_LADDER_TOKENS") {
            None => DEFAULT_BILLING_LADDER_TOKENS.to_vec(),
            Some(raw) => {
                let mut ladder = Vec::new();
                for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    ladder.push(part.parse::<u64>().map_err(|e| ConfigError::Invalid {
                        name: "BILLING_LADDER_TOKENS",
                        reason: format!("{part:?} 不是整数：{e}"),
                    })?);
                }
                if ladder.windows(2).any(|w| w[0] >= w[1]) {
                    return Err(ConfigError::Invalid {
                        name: "BILLING_LADDER_TOKENS",
                        reason: "阈值必须严格递增（否则档位边界无法判定）".into(),
                    });
                }
                ladder
            }
        };

        let relay_max_body_bytes = match lookup("RELAY_MAX_BODY_BYTES") {
            None => DEFAULT_RELAY_MAX_BODY_BYTES,
            Some(raw) => raw
                .trim()
                .parse::<usize>()
                .map_err(|e| ConfigError::Invalid {
                    name: "RELAY_MAX_BODY_BYTES",
                    reason: e.to_string(),
                })?,
        };
        // 上限为 0 会让所有中转请求都被拒（含图片的请求必然超限），属于配错，直接拒绝启动
        if relay_max_body_bytes == 0 {
            return Err(ConfigError::Invalid {
                name: "RELAY_MAX_BODY_BYTES",
                reason: "必须大于 0".into(),
            });
        }

        let relay_max_concurrent_per_user = match lookup("RELAY_MAX_CONCURRENT_PER_USER") {
            None => DEFAULT_RELAY_MAX_CONCURRENT_PER_USER,
            Some(raw) => raw
                .trim()
                .parse::<i32>()
                .map_err(|e| ConfigError::Invalid {
                    name: "RELAY_MAX_CONCURRENT_PER_USER",
                    reason: e.to_string(),
                })?,
        };
        if relay_max_concurrent_per_user < 1 {
            return Err(ConfigError::Invalid {
                name: "RELAY_MAX_CONCURRENT_PER_USER",
                reason: "必须大于等于 1".into(),
            });
        }

        let security = Self::parse_security(&lookup)?;

        Ok(Self {
            bind_addr,
            database_url,
            db_max_connections,
            log_level,
            deepseek_api_key: lookup("DEEPSEEK_API_KEY").unwrap_or_default(),
            deepseek_base_url: lookup("DEEPSEEK_BASE_URL")
                .map(|v| v.trim().trim_end_matches('/').to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_DEEPSEEK_BASE_URL.to_string()),
            deepseek_model: lookup("DEEPSEEK_MODEL")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_DEEPSEEK_MODEL.to_string()),
            billing_ladder_tokens,
            relay_max_body_bytes,
            relay_max_concurrent_per_user,
            token_ttl_days,
            admin_email,
            admin_password,
            security,
        })
    }

    /// 安全护栏配置（一组）。
    ///
    /// 默认取向：**保护上游账号与整体容量，但不误伤正常学生**——所以速率给得宽、
    /// 突发容量等于一分钟的持续速率（学生一轮工具调用会连发数个请求），
    /// 真正偏紧的是"反复登录失败"这条。
    fn parse_security(
        lookup: &impl Fn(&str) -> Option<String>,
    ) -> Result<SecuritySettings, ConfigError> {
        let trust_proxy = match lookup("SECURITY_TRUST_PROXY") {
            None => DEFAULT_TRUST_PROXY,
            Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                other => {
                    return Err(ConfigError::Invalid {
                        name: "SECURITY_TRUST_PROXY",
                        reason: format!("{other:?} 不是布尔值"),
                    });
                }
            },
        };

        Ok(SecuritySettings {
            trust_proxy,
            auth_rate_per_minute: positive_u32(
                lookup,
                "SECURITY_AUTH_RATE_PER_MINUTE",
                DEFAULT_AUTH_RATE_PER_MINUTE,
            )?,
            auth_burst: positive_u32(lookup, "SECURITY_AUTH_BURST", DEFAULT_AUTH_BURST)?,
            relay_rate_per_minute: positive_u32(
                lookup,
                "SECURITY_RELAY_RATE_PER_MINUTE",
                DEFAULT_RELAY_RATE_PER_MINUTE,
            )?,
            relay_burst: positive_u32(lookup, "SECURITY_RELAY_BURST", DEFAULT_RELAY_BURST)?,
            login_max_failures: positive_u32(
                lookup,
                "SECURITY_LOGIN_MAX_FAILURES",
                DEFAULT_LOGIN_MAX_FAILURES,
            )?,
            login_failure_window_secs: positive_u64(
                lookup,
                "SECURITY_LOGIN_FAILURE_WINDOW_SECS",
                DEFAULT_LOGIN_FAILURE_WINDOW_SECS,
            )?,
            login_block_secs: positive_u64(
                lookup,
                "SECURITY_LOGIN_BLOCK_SECS",
                DEFAULT_LOGIN_BLOCK_SECS,
            )?,
            max_concurrent_global: positive_i32(
                lookup,
                "SECURITY_MAX_CONCURRENT_GLOBAL",
                DEFAULT_MAX_CONCURRENT_GLOBAL,
            )?,
        })
    }
}

/// 读一个必须为正整数的环境变量（配成 0 通常是漏配，直接拒绝启动而不是静默放宽）。
fn positive_u32(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u32,
) -> Result<u32, ConfigError> {
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    let value = raw
        .trim()
        .parse::<u32>()
        .map_err(|e| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        })?;
    if value == 0 {
        return Err(ConfigError::Invalid {
            name,
            reason: "必须大于 0".into(),
        });
    }
    Ok(value)
}

fn positive_u64(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    let value = raw
        .trim()
        .parse::<u64>()
        .map_err(|e| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        })?;
    if value == 0 {
        return Err(ConfigError::Invalid {
            name,
            reason: "必须大于 0".into(),
        });
    }
    Ok(value)
}

fn positive_i32(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: i32,
) -> Result<i32, ConfigError> {
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    let value = raw
        .trim()
        .parse::<i32>()
        .map_err(|e| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        })?;
    if value <= 0 {
        return Err(ConfigError::Invalid {
            name,
            reason: "必须大于 0".into(),
        });
    }
    Ok(value)
}

/// 加载 `server/.env`（开发期便利）。文件不存在不算错误——生产由 systemd 注入环境变量。
pub fn load_dotenv() {
    match dotenvy::dotenv() {
        Ok(path) => tracing::debug!(path = %path.display(), "已加载 .env"),
        Err(_) => tracing::debug!("未发现 .env，使用进程环境变量"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn database_url_is_required() {
        let err = Config::from_lookup(lookup(&[])).unwrap_err();
        assert!(matches!(err, ConfigError::Missing("DATABASE_URL")));

        // 空白串等同缺失，避免 systemd 里写了空值却以为配好了
        let err = Config::from_lookup(lookup(&[("DATABASE_URL", "   ")])).unwrap_err();
        assert!(matches!(err, ConfigError::Missing("DATABASE_URL")));
    }

    #[test]
    fn optional_vars_have_defaults() {
        let cfg = Config::from_lookup(lookup(&[(
            "DATABASE_URL",
            "postgres://mistake:mistake@127.0.0.1:5432/mistake_agent",
        )]))
        .unwrap();
        assert_eq!(cfg.bind_addr.to_string(), DEFAULT_BIND_ADDR);
        assert_eq!(cfg.db_max_connections, DEFAULT_DB_MAX_CONNECTIONS);
        assert_eq!(cfg.log_level, DEFAULT_LOG_LEVEL);
        assert_eq!(cfg.deepseek_base_url, DEFAULT_DEEPSEEK_BASE_URL);
        assert!(cfg.deepseek_api_key.is_empty());
        assert_eq!(cfg.deepseek_model, DEFAULT_DEEPSEEK_MODEL);
        assert_eq!(cfg.billing_ladder_tokens, DEFAULT_BILLING_LADDER_TOKENS);
        assert_eq!(cfg.token_ttl_days, DEFAULT_TOKEN_TTL_DAYS);
        assert!(cfg.admin_email.is_none());
        assert!(cfg.admin_password.is_none());
        // 安全护栏：默认**不信任**转发头（防伪造 IP 绕过限流）
        assert!(!cfg.security.trust_proxy);
        assert_eq!(
            cfg.security.relay_rate_per_minute,
            DEFAULT_RELAY_RATE_PER_MINUTE
        );
        assert_eq!(cfg.security.relay_burst, DEFAULT_RELAY_BURST);
        assert_eq!(
            cfg.security.auth_rate_per_minute,
            DEFAULT_AUTH_RATE_PER_MINUTE
        );
        assert_eq!(cfg.security.auth_burst, DEFAULT_AUTH_BURST);
        assert_eq!(cfg.security.login_max_failures, DEFAULT_LOGIN_MAX_FAILURES);
        assert_eq!(cfg.security.login_block_secs, DEFAULT_LOGIN_BLOCK_SECS);
        assert_eq!(
            cfg.security.login_failure_window_secs,
            DEFAULT_LOGIN_FAILURE_WINDOW_SECS
        );
        assert_eq!(
            cfg.security.max_concurrent_global,
            DEFAULT_MAX_CONCURRENT_GLOBAL
        );
        assert_eq!(cfg.relay_max_body_bytes, DEFAULT_RELAY_MAX_BODY_BYTES);
        assert_eq!(
            cfg.relay_max_concurrent_per_user,
            DEFAULT_RELAY_MAX_CONCURRENT_PER_USER
        );
    }

    #[test]
    fn security_settings_are_parsed_and_validated() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("SECURITY_TRUST_PROXY", "TRUE"),
            ("SECURITY_RELAY_RATE_PER_MINUTE", "120"),
            ("SECURITY_LOGIN_MAX_FAILURES", "10"),
        ]))
        .unwrap();
        assert!(cfg.security.trust_proxy);
        assert_eq!(cfg.security.relay_rate_per_minute, 120);
        assert_eq!(cfg.security.login_max_failures, 10);

        // 非法布尔值、0 速率都要拒绝启动（静默放宽比启动失败更危险）
        for (name, value) in [
            ("SECURITY_TRUST_PROXY", "maybe"),
            ("SECURITY_RELAY_RATE_PER_MINUTE", "0"),
            ("SECURITY_LOGIN_MAX_FAILURES", "abc"),
            ("SECURITY_MAX_CONCURRENT_GLOBAL", "0"),
        ] {
            let err =
                Config::from_lookup(lookup(&[("DATABASE_URL", "postgres://x"), (name, value)]))
                    .unwrap_err();
            assert!(
                matches!(err, ConfigError::Invalid { name: rejected, .. } if rejected == name),
                "{name}={value} 应被拒绝"
            );
        }
    }

    #[test]
    fn billing_ladder_parsing_and_validation() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("BILLING_LADDER_TOKENS", " 1000, 4000 ,16000 "),
        ]))
        .unwrap();
        assert_eq!(cfg.billing_ladder_tokens, vec![1000, 4000, 16000]);

        // 空串 = 不设阶梯（所有请求都只扣 1 次），需要能表达出来
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("BILLING_LADDER_TOKENS", "  "),
        ]))
        .unwrap();
        assert!(cfg.billing_ladder_tokens.is_empty());

        // 非整数、非递增都要拒绝：档位边界判定依赖严格递增
        for bad in ["abc", "1000,abc", "4000,1000", "1000,1000"] {
            let err = Config::from_lookup(lookup(&[
                ("DATABASE_URL", "postgres://x"),
                ("BILLING_LADDER_TOKENS", bad),
            ]))
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::Invalid {
                        name: "BILLING_LADDER_TOKENS",
                        ..
                    }
                ),
                "{bad} 应被拒绝"
            );
        }
    }

    #[test]
    fn invalid_bind_addr_and_pool_size_are_rejected() {
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("BIND_ADDR", "不是地址"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                name: "BIND_ADDR",
                ..
            }
        ));

        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("DB_MAX_CONNECTIONS", "0"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                name: "DB_MAX_CONNECTIONS",
                ..
            }
        ));
    }

    #[test]
    fn upstream_base_url_is_normalized() {
        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("DEEPSEEK_BASE_URL", " https://example.com/v1/ "),
        ]))
        .unwrap();
        assert_eq!(cfg.deepseek_base_url, "https://example.com/v1");
    }

    #[test]
    fn token_ttl_must_be_positive() {
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("TOKEN_TTL_DAYS", "0"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                name: "TOKEN_TTL_DAYS",
                ..
            }
        ));

        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("TOKEN_TTL_DAYS", "30"),
        ]))
        .unwrap();
        assert_eq!(cfg.token_ttl_days, 30);
    }

    #[test]
    fn admin_seed_requires_both_halves() {
        // 只给邮箱：视为漏配，直接拒绝（而不是静默跳过种子创建）
        let err = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("ADMIN_EMAIL", "admin@example.com"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                name: "ADMIN_EMAIL",
                ..
            }
        ));

        let cfg = Config::from_lookup(lookup(&[
            ("DATABASE_URL", "postgres://x"),
            ("ADMIN_EMAIL", " admin@example.com "),
            ("ADMIN_PASSWORD", "hunter2hunter2"),
        ]))
        .unwrap();
        assert_eq!(cfg.admin_email.as_deref(), Some("admin@example.com"));
        assert_eq!(cfg.admin_password.as_deref(), Some("hunter2hunter2"));
    }
}
