//! 审计（ADR-0017）：默认全覆盖，M1 内存 sink，M2 经 storage 落 JSONL。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::kernel::dispatch::Caller;
use crate::kernel::message::MessageId;
use crate::kernel::session::SessionKey;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum AuditRecord {
    EntryPointCall {
        entry: String,
        caller: Caller,
        ok: bool,
        error: Option<String>,
    },
    MessageCompleted {
        message_id: MessageId,
    },
    MessageEdited {
        message_id: MessageId,
        branch_id: MessageId,
    },
    BranchSwitched {
        message_id: MessageId,
    },
    SessionSwitched {
        from: SessionKey,
        to: SessionKey,
        reason: String,
    },
    MemoryWrite {
        path: String,
    },
    MemoryRead {
        path: Option<String>,
    },
    MemoryRemove {
        path: String,
    },
    SettingsChanged,
    BalanceChecked {
        main_ok: bool,
        vision_ok: bool,
    },
    LlmCall {
        provider: String,
        model: String,
        kind: String,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
        duration_ms: u64,
        ok: bool,
    },
    /// 例外条款：compute 代码与结果全量记录。
    ComputeExec {
        code: String,
        result: String,
    },
    Lifecycle {
        phase: String,
    },
    AccessDenied {
        entry: String,
        caller: Caller,
    },
    TurnEnded {
        stop_reason: String,
        tool_calls: usize,
    },
    Interrupt {
        name: String,
        reason: String,
    },
    Retry {
        entry: String,
        attempt: u32,
    },
    Compaction {
        session: String,
        summarized: usize,
    },
}

/// 审计落盘接口（M1 内存实现；M2 storage JSONL 实现）。
pub trait AuditSink: Send + Sync {
    fn append(&self, record: AuditRecord);
}

/// 审计器：内核组件统一入口。
#[derive(Clone)]
pub struct Auditor {
    sink: Arc<dyn AuditSink>,
}

impl Auditor {
    pub fn new(sink: Arc<dyn AuditSink>) -> Self {
        Self { sink }
    }

    pub fn record(&self, record: AuditRecord) {
        self.sink.append(record);
    }
}

/// M1 内存 sink：测试可断言。
#[derive(Default)]
pub struct MemoryAuditSink {
    records: std::sync::Mutex<Vec<AuditRecord>>,
}

impl MemoryAuditSink {
    pub fn take(&self) -> Vec<AuditRecord> {
        std::mem::take(&mut *self.records.lock().expect("audit poisoned"))
    }
}

impl AuditSink for MemoryAuditSink {
    fn append(&self, record: AuditRecord) {
        self.records.lock().expect("audit poisoned").push(record);
    }
}
