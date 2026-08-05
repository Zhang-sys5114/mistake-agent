//! 分级诊断日志（ADR-0018 修订版：五级 + PANIC 标记，flexi_logger 实现）。

use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;

/// 诊断日志级别：DEBUG < INFO < WARN < ERROR < CRITICAL。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl Level {
    fn spec(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error | Level::Critical => "error",
        }
    }
}

/// 日志门面：内部委托 log + flexi_logger。
#[derive(Debug, Clone, Default)]
pub struct Logger;

impl Logger {
    /// 初始化全局日志后端（数据根目录 logs/，10MB 大小轮转）。
    pub fn init(level: Level, dir: &Path) -> Result<(), String> {
        static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
        INITIALIZED.get_or_init(|| init_once(level, dir)).clone()
    }

    /// 运行时切换日志级别（set_settings 保存 log_level 后调用；失败静默，下次启动仍生效）。
    pub fn set_level(level: Level) {
        static HANDLE: OnceLock<Option<flexi_logger::LoggerHandle>> = OnceLock::new();
        if let Some(handle) = HANDLE.get().and_then(|h| h.as_ref())
            && let Ok(spec) = flexi_logger::LogSpecification::parse(level.spec())
        {
            handle.set_new_spec(spec);
        }
    }

    pub fn log(&self, level: Level, message: &str) {
        match level {
            Level::Debug => log::debug!("{}", message),
            Level::Info => log::info!("{}", message),
            Level::Warn => log::warn!("{}", message),
            Level::Error => log::error!("{}", message),
            // CRITICAL / PANIC 映射到 ERROR 级 + 标记（ADR-0018 修订）。
            Level::Critical => log::error!("[CRITICAL] {}", message),
        }
    }

    pub fn debug(&self, m: &str) {
        self.log(Level::Debug, m);
    }
    pub fn info(&self, m: &str) {
        self.log(Level::Info, m);
    }
    pub fn warn(&self, m: &str) {
        self.log(Level::Warn, m);
    }
    pub fn error(&self, m: &str) {
        self.log(Level::Error, m);
    }
    pub fn critical(&self, m: &str) {
        self.log(Level::Critical, m);
    }
    /// panic hook 用：CRITICAL + [PANIC] 标记后退出。
    pub fn panic(&self, m: &str) {
        log::error!("[PANIC] {}", m);
    }
}

fn init_once(level: Level, dir: &Path) -> Result<(), String> {
    use flexi_logger::{
        Cleanup, Criterion, Duplicate, FileSpec, Logger as FlLogger, Naming, WriteMode,
    };

    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let handle = FlLogger::try_with_str(level.spec())
        .map_err(|e| e.to_string())?
        .log_to_file(FileSpec::default().directory(dir))
        .rotate(
            Criterion::Size(10_000_000),
            Naming::Numbers,
            Cleanup::KeepLogFiles(5),
        )
        // 双输出：WARN 及以上同步重复到 stderr（参考 ret2shell 多输出；不写 stdout，
        // stdout 不承载日志，避免与 GUI 进程输出混淆）。
        .duplicate_to_stderr(Duplicate::Warn)
        .write_mode(WriteMode::BufferAndFlush)
        .start()
        .map_err(|e| e.to_string())?;
    // 保存句柄供运行时 set_level 使用（重复 init 只执行一次）。
    static HANDLE: OnceLock<Option<flexi_logger::LoggerHandle>> = OnceLock::new();
    let _ = HANDLE.set(Some(handle));
    Ok(())
}

pub type LoggerHandle = Arc<Logger>;

/// 敏感值脱敏（ADR-0018：API key、令牌等一律脱敏）。
pub fn redact_secret(value: &str) -> String {
    const MASK: &str = "****";
    if value.len() <= 8 {
        MASK.to_string()
    } else {
        let head: String = value.chars().take(4).collect();
        let tail: String = value
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}{MASK}{tail}")
    }
}
