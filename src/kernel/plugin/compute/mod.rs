//! compute 服务：契约 + BridgeCompute（kernel → GUI WebView 内 Pyodide 的 RPC 桥接）。
//! 内核插件信息：ServiceId::Compute；能力 = Python 验算（WASM 沙箱，超时/取消/审计在 kernel 侧）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::KernelContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::events::{Event, EventSink};
use crate::kernel::plugin::services::{
    AbortSignal, ComputeError, ComputeHandle, ComputeRequest, ComputeResult, ComputeService,
    ServiceId,
};
use crate::kernel::registry::{KernelDescriptor, KernelPlugin};

/// M1 占位实现：真实执行端是 GUI WebView 内 Pyodide（M4），用于无法回执时的兜底。
pub struct UnavailableCompute;

#[async_trait]
impl ComputeService for UnavailableCompute {
    async fn run(
        &self,
        _request: &ComputeRequest,
        _signal: &AbortSignal,
    ) -> Result<ComputeResult, ComputeError> {
        Err(ComputeError::BackendUnavailable)
    }
}

/// 桥接实现：把代码经 Event::ComputeRequest 发给 GUI，等待 Method::ComputeResult 回执。
/// 带超时与取消；执行端未连接时由 EventSink 静默丢弃，等待超时后返回 Timeout。
pub struct BridgeCompute {
    events: Arc<dyn EventSink>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<ComputeResult>>>>,
    next_id: AtomicU64,
    exec_timeout: Duration,
}

impl BridgeCompute {
    pub fn new(events: Arc<dyn EventSink>) -> Self {
        Self {
            events,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            exec_timeout: Duration::from_secs(180),
        }
    }

    /// GUI 回执入口（Kernel 处理 Method::ComputeResult 时调用）。
    pub fn deliver(&self, id: u64, result: ComputeResult) -> bool {
        if let Some(tx) = self
            .pending
            .lock()
            .expect("compute pending poisoned")
            .remove(&id)
        {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }

    fn drop_pending(&self, id: u64) {
        self.pending
            .lock()
            .expect("compute pending poisoned")
            .remove(&id);
    }
}

#[async_trait]
impl ComputeService for BridgeCompute {
    async fn run(
        &self,
        request: &ComputeRequest,
        signal: &AbortSignal,
    ) -> Result<ComputeResult, ComputeError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("compute pending poisoned")
            .insert(id, tx);
        self.events.emit(Event::ComputeRequest {
            id,
            code: request.code.clone(),
        });

        let cancel = signal.cancelled();
        tokio::select! {
            r = rx => match r {
                Ok(result) => Ok(result),
                Err(_) => {
                    self.drop_pending(id);
                    Err(ComputeError::Transport("执行端未返回结果".into()))
                }
            },
            _ = cancel.cancelled() => {
                self.drop_pending(id);
                Err(ComputeError::Transport("验算被取消".into()))
            }
            _ = tokio::time::sleep(self.exec_timeout) => {
                self.drop_pending(id);
                Err(ComputeError::Timeout)
            }
        }
    }
}

// ---------- 内核插件入口（ADR-0035）：compute::verify ----------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct VerifyParams {
    /// 要执行的 Python 验算代码（Pyodide 沙箱，无文件、无网络）。
    pub code: String,
}

pub struct ComputePlugin;

impl KernelPlugin for ComputePlugin {
    fn info() -> Info {
        Info {
            namespace: "compute".into(),
            provides: vec![ServiceId::Compute],
            tools: vec![ToolDef {
                name: "verify".into(),
                user_visible: false,
                title: Some("数学验算（高级）".into()),
                group: Some("学习".into()),
                description: "在本地 Pyodide 沙箱（无文件、无网络）中运行 Python 代码并返回 stdout/stderr，用于数学/物理答案验算。内置 numpy（数值计算）与 sympy（符号计算：解方程/求导/积分/单位换算），代码须为完整可执行的 Python 语句，例如 print(17*19) 或 import sympy as sp; x=sp.symbols('x'); print(sp.solve(x**2-5*x+6, x))。"
                    .into(),
                params: schemars::schema_for!(VerifyParams),
                policy: CallerPolicy::UserAndModel,
                timeout: Some(200),
                icon: Some("mdi:calculator-variant".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError> {
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

pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<ComputePlugin>()
}

async fn verify_handler(
    compute: ComputeHandle,
    signal: AbortSignal,
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
    use crate::kernel::events::MemoryEventSink;

    #[tokio::test]
    async fn bridge_delivers_result_by_id() {
        let events = Arc::new(MemoryEventSink::default());
        let bridge = Arc::new(BridgeCompute::new(events.clone()));
        let sink = events.clone();
        let bridge2 = bridge.clone();
        let handle = tokio::spawn(async move {
            bridge2
                .run(&ComputeRequest { code: "1+1".into() }, &AbortSignal::new())
                .await
        });
        // 等待事件发出，取 id 后回执。
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut events = sink.take();
        while !events
            .iter()
            .any(|e| matches!(e, Event::ComputeRequest { .. }))
        {
            if tokio::time::Instant::now() >= deadline {
                panic!("compute request 事件未发出");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            events = sink.take();
        }
        let (id, code) = events
            .into_iter()
            .find_map(|e| match e {
                Event::ComputeRequest { id, code } => Some((id, code)),
                _ => None,
            })
            .expect("有 compute request 事件");
        assert_eq!(code, "1+1");
        assert!(bridge.deliver(
            id,
            ComputeResult {
                stdout: "2".into(),
                stderr: String::new(),
                duration_ms: 1
            }
        ));
        let result = handle.await.unwrap().expect("run 应成功");
        assert_eq!(result.stdout, "2");
    }

    #[tokio::test]
    async fn bridge_cancel_drops_pending() {
        let events = Arc::new(MemoryEventSink::default());
        let bridge = Arc::new(BridgeCompute::new(events.clone()));
        let signal = AbortSignal::new();
        let signal2 = signal.clone();
        let bridge2 = bridge.clone();
        let handle = tokio::spawn(async move {
            bridge2
                .run(&ComputeRequest { code: "1".into() }, &signal2)
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if events
                .take()
                .iter()
                .any(|e| matches!(e, Event::ComputeRequest { .. }))
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("compute request 事件未发出");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        signal.cancel();
        let result = handle.await.unwrap();
        assert!(matches!(result, Err(ComputeError::Transport(_))));
    }

    // ---------- 入口 handler 测试（原 plugin/compute 迁移，ADR-0035） ----------

    struct FakeCompute;

    #[async_trait]
    impl ComputeService for FakeCompute {
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
        let handle = ComputeHandle::new(Arc::new(FakeCompute));
        let out = verify_handler(handle, AbortSignal::new(), json!({"code": "print(3+5)"}))
            .await
            .unwrap();
        assert_eq!(out["stdout"], "8");
        assert_eq!(out["ok"], true);
    }

    #[tokio::test]
    async fn backend_unavailable_maps_to_handler_error() {
        let handle = ComputeHandle::new(Arc::new(FakeCompute));
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
        let handle = ComputeHandle::new(Arc::new(FakeCompute));
        let err = verify_handler(handle, AbortSignal::new(), json!({"code": "  "}))
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            crate::kernel::contract::ToolErrorCode::InvalidParams
        );
    }
}
