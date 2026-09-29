//! Mistake Agent 服务端进程入口（ADR-0047）。
//!
//! 启动顺序：配置 → 日志 → 数据库 → 迁移 → HTTP。任一步失败即退出（fail-fast），
//! 失败原因同时走 stderr —— 配置/日志阶段就失败时，日志子系统可能还没就绪。

use std::process::ExitCode;
use std::sync::Arc;

use mistake_agent_server::{auth, config, db, http, logging};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            eprintln!("mistake-agent-server 启动失败：{reason}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    config::load_dotenv();
    let cfg = config::Config::from_env().map_err(|e| e.to_string())?;
    logging::init(&cfg.log_level)?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        bind_addr = %cfg.bind_addr,
        "mistake-agent-server 启动"
    );
    if cfg.deepseek_api_key.is_empty() {
        tracing::warn!("DEEPSEEK_API_KEY 未配置：S1 不校验，S3 中转上线后必需");
    }

    let pool = db::connect(&cfg.database_url, cfg.db_max_connections)
        .await
        .map_err(|e| {
            format!(
                "数据库连接失败（{}）：{e}",
                logging::redact_dsn(&cfg.database_url)
            )
        })?;
    db::migrate(&pool)
        .await
        .map_err(|e| format!("数据库迁移失败：{e}"))?;
    tracing::info!(
        max_connections = cfg.db_max_connections,
        "数据库就绪，迁移已应用"
    );

    // 管理员种子（ADR-0047 决策 3）：仅在配置了 ADMIN_EMAIL/ADMIN_PASSWORD 时才可能创建
    auth::bootstrap_admin(&pool, &cfg).await?;

    let listener = tokio::net::TcpListener::bind(cfg.bind_addr)
        .await
        .map_err(|e| format!("监听 {} 失败：{e}", cfg.bind_addr))?;
    tracing::info!("HTTP 已启动");

    let state = http::AppState::new(pool, Arc::new(cfg));
    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("HTTP 服务异常退出：{e}"))?;

    tracing::info!("已优雅停止");
    Ok(())
}

/// 停止信号：Linux 上 systemd 发 SIGTERM、交互式 Ctrl+C 发 SIGINT，两者都要优雅停止。
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => tracing::info!("收到 SIGINT，优雅停止"),
                    _ = term.recv() => tracing::info!("收到 SIGTERM，优雅停止"),
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "SIGTERM 监听注册失败，仅监听 Ctrl+C");
                if tokio::signal::ctrl_c().await.is_ok() {
                    tracing::info!("收到 SIGINT，优雅停止");
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        match tokio::signal::ctrl_c().await {
            Ok(()) => tracing::info!("收到 Ctrl+C，优雅停止"),
            Err(e) => tracing::error!(error = %e, "停止信号监听失败"),
        }
    }
}
