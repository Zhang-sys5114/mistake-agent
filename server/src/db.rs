//! 数据库：连接池与迁移（ADR-0047 决策 2）。
//!
//! 迁移经 `sqlx::migrate!` **编译期嵌入** `migrations/`：部署物是单二进制，
//! 不依赖工作目录里存在迁移文件（systemd 的 WorkingDirectory 变化不会导致漏迁移）。

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// 获取连接的超时：数据库不可达时快速失败，而不是把启动挂死。
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// 连接池参数集中一处：生产（`connect`）与测试（懒池）共用，避免两处配置漂移。
pub fn pool_options(max_connections: u32) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
}

pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    pool_options(max_connections).connect(database_url).await
}

/// 应用迁移。幂等：sqlx 用 `_sqlx_migrations` 记录已应用版本，重复调用无副作用。
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/// 就绪探针用：数据库是否可达。
pub async fn ping(pool: &PgPool) -> bool {
    sqlx::query("SELECT 1").execute(pool).await.is_ok()
}
