//! compute 插件：验算执行（Python 代码经 GUI WebView 内 Pyodide 沙箱运行）。

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::dispatch::ToolCallContext;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};
use crate::kernel::services::{ComputeError, ComputeHandle, ComputeRequest, ServiceId};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct VerifyParams {
    /// 要执行的 Python 验算代码（Pyodide 沙箱，无文件、无网络）。
    pub code: String,
}

pub struct ComputePlugin;

impl UserPlugin for ComputePlugin {
    fn info() -> Info {
        Info {
            namespace: "compute".into(),
            requires: vec![ServiceId::Compute],
            tools: vec![ToolDef {
                name: "verify".into(),
                user_visible: false,
                title: Some("数学验算（高级）".into()),
                group: Some("学习".into()),
                description: "运行 Python 代码验算数学/物理答案（本地 Pyodide 沙箱，无文件/网络）。用法：compute::verify <Python 代码>"
                    .into(),
                params: schemars::schema_for!(VerifyParams),
                policy: CallerPolicy::UserAndModel,
                timeout: Some(200),
                icon: Some("mdi:calculator-variant".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        let compute = ctx
            .handles
            .compute()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Compute 句柄".into()))?;
        ctx.registrar.tool(
            "verify",
            Arc::new(move |call_ctx: &ToolCallContext, params: Value| {
                let compute = compute.clone();
                let signal = call_ctx.signal.clone();
                Box::pin(async move { verify_handler(compute, signal, params).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<ComputePlugin>()
}

async fn verify_handler(
    compute: ComputeHandle,
    signal: crate::kernel::services::AbortSignal,
    params: Value,
) -> Result<Value, ToolError> {
    let p: VerifyParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    if p.code.trim().is_empty() {
        return Err(ToolError::invalid_params("code 不能为空"));
    }
    let request = ComputeRequest { code: p.code };
    match compute.run(&request, &signal).await {
        Ok(r) => Ok(json!({
            "stdout": r.stdout,
            "stderr": r.stderr,
            "duration_ms": r.duration_ms,
            "ok": r.stderr.trim().is_empty(),
        })),
        Err(ComputeError::BackendUnavailable) => Err(ToolError::handler(
            "验算执行端（Pyodide）未连接，请确认应用已加载完成后再试",
        )),
        Err(ComputeError::Timeout) => Err(ToolError::timeout()),
        Err(e) => Err(ToolError::handler(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::services::{AbortSignal, ComputeResult};

    struct FakeCompute;

    #[async_trait::async_trait]
    impl crate::kernel::services::ComputeService for FakeCompute {
        async fn run(
            &self,
            request: &ComputeRequest,
            _signal: &AbortSignal,
        ) -> Result<ComputeResult, ComputeError> {
            if request.code.contains("raise") {
                return Err(ComputeError::BackendUnavailable);
            }
            Ok(ComputeResult {
                stdout: "8".into(),
                stderr: String::new(),
                duration_ms: 3,
            })
        }
    }

    #[tokio::test]
    async fn verify_returns_structured_result() {
        let handle = ComputeHandle::new(std::sync::Arc::new(FakeCompute));
        let out = verify_handler(handle, AbortSignal::new(), json!({"code": "print(3+5)"}))
            .await
            .unwrap();
        assert_eq!(out["stdout"], "8");
        assert_eq!(out["ok"], true);
    }

    #[tokio::test]
    async fn backend_unavailable_maps_to_handler_error() {
        let handle = ComputeHandle::new(std::sync::Arc::new(FakeCompute));
        let err = verify_handler(
            handle,
            AbortSignal::new(),
            json!({"code": "raise RuntimeError"}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.code,
            crate::kernel::contract::ToolErrorCode::HandlerError
        );
        assert!(err.message.contains("Pyodide"));
    }

    #[tokio::test]
    async fn empty_code_rejected() {
        let handle = ComputeHandle::new(std::sync::Arc::new(FakeCompute));
        let err = verify_handler(handle, AbortSignal::new(), json!({"code": "  "}))
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            crate::kernel::contract::ToolErrorCode::InvalidParams
        );
    }
}
