//! compute 服务：契约 + BridgeCompute（kernel → GUI WebView 内 Pyodide 的 RPC 桥接）。
//! 内核插件信息：ServiceId::Compute；能力 = Python 验算（WASM 沙箱，超时/取消/审计在 kernel 侧）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::kernel::events::{Event, EventSink};
use crate::kernel::services::{
    AbortSignal, ComputeError, ComputeRequest, ComputeResult, ComputeService,
};

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
}
