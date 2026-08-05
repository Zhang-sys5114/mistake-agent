//! storage 服务：M1 内存实现 + M2 文件持久化（会话 JSONL / 错题 JSON / 审计 JSONL 轮转）。

use std::collections::HashMap;

use async_trait::async_trait;

use super::file::FileStorage;
use super::mem::MemoryStorage;
use crate::kernel::agent::session::{Goal, SessionKey, SessionMeta, SessionStatus};
use crate::kernel::audit::{AuditRecord, AuditSink};
use crate::kernel::message::{Message, MessageId};
use crate::kernel::plugin::services::{
    Mistake, MistakeFilter, MistakeId, MistakePatch, MistakeStore, SessionStore, StorageError,
    StorageService,
};

#[derive(Clone)]
pub enum AnyStorage {
    File(FileStorage),
    Mem(MemoryStorage),
}

#[async_trait]
impl SessionStore for AnyStorage {
    async fn create_session(
        &self,
        key: &SessionKey,
        meta: &SessionMeta,
    ) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.create_session(key, meta).await,
            AnyStorage::Mem(s) => s.create_session(key, meta).await,
        }
    }
    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionMeta>, StorageError> {
        match self {
            AnyStorage::File(s) => s.get_session(key).await,
            AnyStorage::Mem(s) => s.get_session(key).await,
        }
    }
    async fn append_message(&self, key: &SessionKey, msg: &Message) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.append_message(key, msg).await,
            AnyStorage::Mem(s) => s.append_message(key, msg).await,
        }
    }
    async fn read_path(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError> {
        match self {
            AnyStorage::File(s) => s.read_path(key).await,
            AnyStorage::Mem(s) => s.read_path(key).await,
        }
    }
    async fn read_all(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError> {
        match self {
            AnyStorage::File(s) => s.read_all(key).await,
            AnyStorage::Mem(s) => s.read_all(key).await,
        }
    }
    async fn set_active_path(
        &self,
        key: &SessionKey,
        message_id: Option<MessageId>,
    ) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.set_active_path(key, message_id).await,
            AnyStorage::Mem(s) => s.set_active_path(key, message_id).await,
        }
    }
    async fn derive_branch(
        &self,
        key: &SessionKey,
        message_id: MessageId,
        text: &str,
    ) -> Result<Vec<Message>, StorageError> {
        match self {
            AnyStorage::File(s) => s.derive_branch(key, message_id, text).await,
            AnyStorage::Mem(s) => s.derive_branch(key, message_id, text).await,
        }
    }
    async fn switch_branch(
        &self,
        key: &SessionKey,
        message_id: MessageId,
    ) -> Result<Vec<Message>, StorageError> {
        match self {
            AnyStorage::File(s) => s.switch_branch(key, message_id).await,
            AnyStorage::Mem(s) => s.switch_branch(key, message_id).await,
        }
    }
    async fn splice_compaction(
        &self,
        key: &SessionKey,
        summary: &Message,
        tail_start: MessageId,
    ) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.splice_compaction(key, summary, tail_start).await,
            AnyStorage::Mem(s) => s.splice_compaction(key, summary, tail_start).await,
        }
    }
    async fn set_goal(&self, key: &SessionKey, goal: &Goal) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.set_goal(key, goal).await,
            AnyStorage::Mem(s) => s.set_goal(key, goal).await,
        }
    }
    async fn archive(&self, key: &SessionKey) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.archive(key).await,
            AnyStorage::Mem(s) => s.archive(key).await,
        }
    }
    async fn list_sessions(&self) -> Result<Vec<SessionMeta>, StorageError> {
        match self {
            AnyStorage::File(s) => s.list_sessions().await,
            AnyStorage::Mem(s) => s.list_sessions().await,
        }
    }
    async fn set_last_activity(
        &self,
        key: &SessionKey,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.set_last_activity(key, at).await,
            AnyStorage::Mem(s) => s.set_last_activity(key, at).await,
        }
    }
}

#[async_trait]
impl MistakeStore for AnyStorage {
    async fn save(&self, mistake: &Mistake) -> Result<MistakeId, StorageError> {
        match self {
            AnyStorage::File(s) => s.save(mistake).await,
            AnyStorage::Mem(s) => s.save(mistake).await,
        }
    }
    async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError> {
        match self {
            AnyStorage::File(s) => s.get(id).await,
            AnyStorage::Mem(s) => s.get(id).await,
        }
    }
    async fn list(&self, filter: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
        match self {
            AnyStorage::File(s) => s.list(filter).await,
            AnyStorage::Mem(s) => s.list(filter).await,
        }
    }
    async fn update(&self, id: &MistakeId, patch: &MistakePatch) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.update(id, patch).await,
            AnyStorage::Mem(s) => s.update(id, patch).await,
        }
    }
    async fn remove(&self, id: &MistakeId) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.remove(id).await,
            AnyStorage::Mem(s) => s.remove(id).await,
        }
    }
}

impl AuditSink for AnyStorage {
    fn append(&self, record: AuditRecord) {
        match self {
            AnyStorage::File(s) => s.append(record),
            AnyStorage::Mem(s) => s.append(record),
        }
    }
}

impl StorageService for AnyStorage {}

pub fn active_session(metas: &[SessionMeta]) -> Option<SessionMeta> {
    metas
        .iter()
        .find(|m| m.status == SessionStatus::Active)
        .cloned()
}

pub fn last_message_id(messages: &[Message]) -> Option<MessageId> {
    messages.last().map(|m| m.id)
}

/// 沿 parent 链回溯构造活跃路径（根 → 末端）。
/// `active_path` 为 None（旧数据/线性会话）时退化为完整消息列表。
pub fn active_chain(messages: &[Message], active_path: Option<MessageId>) -> Vec<Message> {
    let Some(end) = active_path else {
        return messages.to_vec();
    };
    let by_id: HashMap<MessageId, Message> = messages.iter().map(|m| (m.id, m.clone())).collect();
    if !by_id.contains_key(&end) {
        return messages.to_vec();
    }
    let mut chain = Vec::new();
    let mut cur = Some(end);
    while let Some(id) = cur {
        match by_id.get(&id) {
            Some(m) => {
                cur = m.parent_id;
                chain.push(m.clone());
            }
            None => break,
        }
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::message::MessageKind;

    #[tokio::test]
    async fn mistake_crud_roundtrip() {
        let store = MemoryStorage::new();
        let m = Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: "二次函数".into(),
            question: "求顶点坐标".into(),
            student_answer: "(1,2)".into(),
            reference_answer: Some("(1,2)".into()),
            is_correct: false,
            analysis: "符号错误".into(),
            created_at: chrono::Utc::now(),
        };
        let id = store.save(&m).await.unwrap();
        let got = store.get(&id).await.unwrap().unwrap();
        assert_eq!(got.knowledge_point, "二次函数");
        let list = store
            .list(&MistakeFilter {
                subject: Some("数学".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        store.remove(&id).await.unwrap();
        assert!(store.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_append_and_archive() {
        let store = MemoryStorage::new();
        let key = SessionKey::new();
        let meta = SessionMeta::new(key);
        store.create_session(&key, &meta).await.unwrap();
        let msg = Message {
            id: MessageId::new(),
            parent_id: None,
            kind: MessageKind::User {
                text: "你好".into(),
                display_text: None,
                attachments: vec![],
            },
            created_at: chrono::Utc::now(),
        };
        store.append_message(&key, &msg).await.unwrap();
        assert_eq!(store.read_all(&key).await.unwrap().len(), 1);
        store.archive(&key).await.unwrap();
        assert_eq!(
            store.get_session(&key).await.unwrap().unwrap().status,
            SessionStatus::Archived
        );
    }

    #[tokio::test]
    async fn message_tree_branch_derive_and_switch() {
        let store = MemoryStorage::new();
        let key = SessionKey::new();
        store
            .create_session(&key, &SessionMeta::new(key))
            .await
            .unwrap();

        let u1 = Message::user("第一问");
        let a1 = Message::assistant("回答一");
        let u2 = Message::user("追问");
        let a2 = Message::assistant("回答二");
        let mut prev = None;
        for m in [&u1, &a1, &u2, &a2] {
            let mut m = m.clone();
            m.parent_id = prev;
            prev = Some(m.id);
            store.append_message(&key, &m).await.unwrap();
        }
        assert_eq!(store.read_path(&key).await.unwrap().len(), 4);

        // 编辑 a1 → 派生新分支 [u1, a1']
        let path = store
            .derive_branch(&key, a1.id, "回答一（修订）")
            .await
            .unwrap();
        assert_eq!(path.len(), 2);
        assert!(
            matches!(&path[1].kind, MessageKind::Assistant { text } if text == "回答一（修订）")
        );
        // 历史保留：JSONL 共 5 条，活跃路径只剩 2 条。
        assert_eq!(store.read_all(&key).await.unwrap().len(), 5);
        assert_eq!(store.read_path(&key).await.unwrap().len(), 2);

        // 不能编辑 user 消息。
        assert!(store.derive_branch(&key, u1.id, "改了").await.is_err());

        // 切回 a2 分支（4 条原始路径）。
        let path2 = store.switch_branch(&key, a2.id).await.unwrap();
        assert_eq!(path2.len(), 4);
        assert_eq!(path2[3].id, a2.id);
        assert_eq!(store.read_path(&key).await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn compaction_splice_relinks_tail_and_appends_summary() {
        let store = MemoryStorage::new();
        let key = SessionKey::new();
        store
            .create_session(&key, &SessionMeta::new(key))
            .await
            .unwrap();
        let m1 = Message::user("旧 1");
        let m2 = Message::user("旧 2");
        let m3 = Message::user("保留 3");
        let mut prev = None;
        for m in [&m1, &m2, &m3] {
            let mut m = m.clone();
            m.parent_id = prev;
            prev = Some(m.id);
            store.append_message(&key, &m).await.unwrap();
        }

        let summary = Message::system("上下文压缩摘要：…");
        store
            .splice_compaction(&key, &summary, m3.id)
            .await
            .unwrap();
        let all = store.read_all(&key).await.unwrap();
        assert_eq!(all.len(), 4, "摘要追加，旧消息全量保留");
        assert_eq!(summary.parent_id, None, "摘要是新活跃路径根");
        let tail = all.iter().find(|m| m.id == m3.id).unwrap();
        assert_eq!(tail.parent_id, Some(summary.id), "保留段首条改挂摘要");

        store.set_active_path(&key, Some(m3.id)).await.unwrap();
        let path = store.read_path(&key).await.unwrap();
        assert_eq!(path.len(), 2, "活跃路径 = 摘要 + 保留段");
        assert_eq!(path[0].id, summary.id);
        assert_eq!(path[1].id, m3.id);
    }
}
