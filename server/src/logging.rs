//! 分级日志与脱敏（ADR-0047 决策 2/10）。
//!
//! 与客户端（flexi_logger + `log`）实现独立，但分级语义对齐：级别由 `LOG_LEVEL` 给出，
//! 存在 `RUST_LOG` 时以 `RUST_LOG` 为准（tracing 生态惯例，便于临时排障）。
//!
//! 纪律（S3 中转落地时逐条兑现）：
//! - 中转请求/响应正文**永不进日志**（ADR-0047 决策 5/10）
//! - 平台密钥、用户令牌、口令等敏感值不打印（只打印"是否已配置"）
//! - 连接串一律经 [`redact_dsn`] 处理后再进日志

use tracing_subscriber::EnvFilter;

/// 初始化全局订阅者。仅在进程入口调用一次（重复调用会 panic，测试不调用）。
pub fn init(default_level: &str) -> Result<(), String> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_level))
        .map_err(|e| format!("日志级别非法（LOG_LEVEL={default_level}）：{e}"))?;

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        .init();
    Ok(())
}

/// 连接串脱敏：保留用户名与主机（便于排查连的是哪个库），**隐藏口令**。
///
/// 无凭据或无口令的连接串原样返回——没有可泄漏的内容，不该被无谓改写。
pub fn redact_dsn(dsn: &str) -> String {
    let Some(scheme_end) = dsn.find("://") else {
        return dsn.to_string();
    };
    let creds_start = scheme_end + 3;
    let Some(at_rel) = dsn[creds_start..].find('@') else {
        return dsn.to_string();
    };
    let at = creds_start + at_rel;
    let creds = &dsn[creds_start..at];
    let Some((user, _password)) = creds.split_once(':') else {
        return dsn.to_string();
    };
    if user.is_empty() {
        return format!("{}://***{}", &dsn[..scheme_end], &dsn[at..]);
    }
    format!("{}://{}:***{}", &dsn[..scheme_end], user, &dsn[at..])
}

#[cfg(test)]
mod tests {
    use super::redact_dsn;

    #[test]
    fn redacts_password_but_keeps_user_and_host() {
        assert_eq!(
            redact_dsn("postgres://mistake:secret@127.0.0.1:5432/mistake_agent"),
            "postgres://mistake:***@127.0.0.1:5432/mistake_agent"
        );
    }

    #[test]
    fn leaves_dsns_without_password_untouched() {
        for dsn in [
            "postgres://mistake@127.0.0.1:5432/mistake_agent",
            "postgres://127.0.0.1:5432/mistake_agent",
            "不是连接串",
        ] {
            assert_eq!(redact_dsn(dsn), dsn, "{dsn} 不该被改写");
        }
    }

    #[test]
    fn redacts_password_when_user_is_empty() {
        // 用户名为空但带了口令：口令照样是敏感的，整体隐去
        assert_eq!(
            redact_dsn("postgres://:secret@host/db"),
            "postgres://***@host/db"
        );
    }
}
