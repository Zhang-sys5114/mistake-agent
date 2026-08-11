//! Compute 契约（Q10）。

use super::*;

// ---------- Compute 契约（Q10） ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeRequest {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeResult {
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ComputeError {
    #[error("执行端不可用（GUI/Pyodide 未连接）")]
    BackendUnavailable,
    #[error("执行超时")]
    Timeout,
    #[error("超出资源限制：{0}")]
    ResourceLimit(String),
    #[error("传输错误：{0}")]
    Transport(String),
}

/// 代码执行失败（Python traceback）走结果，不走错误（Q10）。
#[async_trait]
pub trait ComputeService: Send + Sync {
    async fn run(
        &self,
        request: &ComputeRequest,
        signal: &AbortSignal,
    ) -> Result<ComputeResult, ComputeError>;
}

#[derive(Clone)]
pub struct ComputeHandle {
    inner: Arc<dyn ComputeService>,
}

impl ComputeHandle {
    pub fn new(inner: Arc<dyn ComputeService>) -> Self {
        Self { inner }
    }
    pub async fn run(
        &self,
        request: &ComputeRequest,
        signal: &AbortSignal,
    ) -> Result<ComputeResult, ComputeError> {
        self.inner.run(request, signal).await
    }
}
