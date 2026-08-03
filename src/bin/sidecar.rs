//! Kernel 进程入口：stdio JSONL RPC（ADR-0013 / Q15）。
//! GUI 经 Tauri 拉起本进程，双方一行一个 JSON 帧通信。

use std::sync::Arc;
use std::sync::Mutex;

use tokio::io::{AsyncBufReadExt, BufReader};

use mistake_agent::kernel::events::{Event, EventSink};
use mistake_agent::kernel::rpc::{Kernel, RpcFrame, RpcRequest};

/// 事件 → stdout 帧（fire-and-forget，逐行 flush 保证打字机效果）。
struct StdioEventSink {
    lock: Arc<Mutex<()>>,
}

impl EventSink for StdioEventSink {
    fn emit(&self, event: Event) {
        let frame = RpcFrame::Event { event };
        write_frame(&self.lock, &frame);
    }
}

fn write_frame(lock: &Arc<Mutex<()>>, frame: &RpcFrame) {
    use std::io::Write;
    let Ok(line) = serde_json::to_string(frame) else {
        return;
    };
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[PANIC] {info}");
        log::error!("[PANIC] {info}");
    }));

    let out_lock = Arc::new(Mutex::new(()));
    let events: Arc<dyn EventSink> = Arc::new(StdioEventSink {
        lock: out_lock.clone(),
    });
    let kernel = Kernel::new(events).await?;

    let mut join_set = tokio::task::JoinSet::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: RpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("请求解析失败：{e}");
                continue;
            }
        };
        let kernel = kernel.clone();
        let out_lock = out_lock.clone();
        let id = request.id;
        join_set.spawn(async move {
            let frame = match kernel.handle(request).await {
                Ok(Some(frame)) => frame,
                Ok(None) => return,
                Err(e) => RpcFrame::Response {
                    id,
                    result: None,
                    error: Some(e),
                },
            };
            write_frame(&out_lock, &frame);
        });
    }
    // stdin 关闭（如管道测试）后，等所有在途请求收尾，并轮询 kernel 直到回合结束再退出
    //（回合任务独立于 JoinSet，运行时必须先于其存活）。
    while join_set.join_next().await.is_some() {}
    while !kernel.is_idle().await {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(())
}
