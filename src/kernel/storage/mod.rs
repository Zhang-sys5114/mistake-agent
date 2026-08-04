//! storage 内核插件（Linux 内核组织风格）。
//!
//! 插件信息：
//! - ServiceId::Storage（内核插件，信任边界内）
//! - 能力：会话持久化（SessionStore）、错题本（MistakeStore）、审计落盘（AuditSink）
//! - 实现拆分：`mem.rs` 内存后端 / `file.rs` 文件后端（JSONL+JSON+轮转）/ `core.rs` AnyStorage 路由与消息树链
//! - 共享内部状态：`Inner`（本文件，两个后端共用）

use std::collections::HashMap;

use crate::kernel::audit::AuditRecord;
use crate::kernel::message::Message;
use crate::kernel::services::Mistake;
use crate::kernel::session::{SessionKey, SessionMeta};

mod core;
mod file;
mod mem;

/// 共享内存态：会话/消息/错题/审计（内存后端与文件后端共用同一结构）。
#[derive(Default)]
pub(crate) struct Inner {
    pub(crate) sessions: HashMap<SessionKey, SessionMeta>,
    pub(crate) messages: HashMap<SessionKey, Vec<Message>>,
    pub(crate) mistakes: Vec<Mistake>,
    pub(crate) audit: Vec<AuditRecord>,
}

pub use core::{AnyStorage, active_chain, active_session, last_message_id};
pub use file::FileStorage;
pub use mem::MemoryStorage;
