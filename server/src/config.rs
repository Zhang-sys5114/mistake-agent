//! 服务端配置（ADR-0047 决策 2）：**环境变量是唯一来源**，启动时一次性读取并校验（fail-fast）。
//!
//! 生产经 systemd `EnvironmentFile` 注入；开发期由 `load_dotenv()` 读 `server/.env`（缺失不算错误）。
//! 解析逻辑走 `from_lookup`，把"取值来源"与"解析校验"分开，使校验规则可测（不依赖进程环境变量）。

use std::env;
use std::net::SocketAddr;

use thiserror::Error;

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
}

/// 默认监听回环：TLS 由前置反向代理终结，服务端不直接对外（ADR-0047 决策 2）。
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

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
        })
    }
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
}
