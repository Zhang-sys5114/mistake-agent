//! storage 服务：M1 内存实现 + M2 文件持久化（会话 JSONL / 错题 JSON / 审计 JSONL 轮转）。

use std::collections::HashMap;

use async_trait::async_trait;

use super::file::FileStorage;
use super::mem::MemoryStorage;
use crate::kernel::agent::session::{Goal, SessionKey, SessionMeta, SessionStatus};
use crate::kernel::audit::{AuditRecord, AuditSink};
use crate::kernel::message::{Message, MessageId};
use crate::kernel::plugin::services::{
    Domain, DomainIo, Mistake, MistakeFilter, MistakeId, MistakePatch, MistakeStore, RelPath,
    SessionStore, StorageError, StorageService, TmpIo,
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
            // UFCS：消除与 DomainIo::list 的方法名冲突。
            AnyStorage::File(s) => MistakeStore::list(s, filter).await,
            AnyStorage::Mem(s) => MistakeStore::list(s, filter).await,
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
            // UFCS：消除与 DomainIo::remove 的方法名冲突。
            AnyStorage::File(s) => MistakeStore::remove(s, id).await,
            AnyStorage::Mem(s) => MistakeStore::remove(s, id).await,
        }
    }
    async fn remove_many(&self, ids: &[MistakeId]) -> Result<usize, StorageError> {
        match self {
            AnyStorage::File(s) => s.remove_many(ids).await,
            AnyStorage::Mem(s) => s.remove_many(ids).await,
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

#[async_trait]
impl DomainIo for AnyStorage {
    async fn read(&self, domain: Domain, rel: &RelPath) -> Result<Vec<u8>, StorageError> {
        match self {
            AnyStorage::File(s) => s.read(domain, rel).await,
            AnyStorage::Mem(s) => s.read(domain, rel).await,
        }
    }
    async fn write(&self, domain: Domain, rel: &RelPath, bytes: &[u8]) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.write(domain, rel, bytes).await,
            AnyStorage::Mem(s) => s.write(domain, rel, bytes).await,
        }
    }
    async fn remove(&self, domain: Domain, rel: &RelPath) -> Result<(), StorageError> {
        match self {
            // UFCS：消除与 MistakeStore::remove 的方法名冲突。
            AnyStorage::File(s) => DomainIo::remove(s, domain, rel).await,
            AnyStorage::Mem(s) => DomainIo::remove(s, domain, rel).await,
        }
    }
    async fn remove_tree(&self, domain: Domain, rel: &RelPath) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => DomainIo::remove_tree(s, domain, rel).await,
            AnyStorage::Mem(s) => DomainIo::remove_tree(s, domain, rel).await,
        }
    }
    async fn list(&self, domain: Domain) -> Result<Vec<String>, StorageError> {
        match self {
            // UFCS：消除与 MistakeStore::list 的方法名冲突。
            AnyStorage::File(s) => DomainIo::list(s, domain).await,
            AnyStorage::Mem(s) => DomainIo::list(s, domain).await,
        }
    }
    async fn read_legacy(&self, domain: Domain, legacy_rel: &str) -> Result<Vec<u8>, StorageError> {
        match self {
            AnyStorage::File(s) => DomainIo::read_legacy(s, domain, legacy_rel).await,
            AnyStorage::Mem(s) => DomainIo::read_legacy(s, domain, legacy_rel).await,
        }
    }
    async fn remove_legacy(&self, domain: Domain, legacy_rel: &str) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => DomainIo::remove_legacy(s, domain, legacy_rel).await,
            AnyStorage::Mem(s) => DomainIo::remove_legacy(s, domain, legacy_rel).await,
        }
    }
}

#[async_trait]
impl TmpIo for AnyStorage {
    async fn read_staged(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        match self {
            AnyStorage::File(s) => s.read_staged(path).await,
            AnyStorage::Mem(s) => s.read_staged(path).await,
        }
    }
    async fn remove_staged(&self, path: &str) -> Result<(), StorageError> {
        match self {
            AnyStorage::File(s) => s.remove_staged(path).await,
            AnyStorage::Mem(s) => s.remove_staged(path).await,
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
    use std::sync::Arc;

    #[tokio::test]
    async fn mistake_crud_roundtrip() {
        let store: Arc<dyn MistakeStore> = Arc::new(MemoryStorage::new());
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
            pinned: false,
            deleted_at: None,
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
    async fn mistake_soft_delete_pin_and_batch_remove() {
        let store: Arc<dyn MistakeStore> = Arc::new(MemoryStorage::new());
        let m1 = Mistake {
            id: MistakeId(uuid::Uuid::new_v4()),
            subject: "数学".into(),
            knowledge_point: "绝对值".into(),
            question: "|-3| = ?".into(),
            student_answer: "-3".into(),
            reference_answer: Some("3".into()),
            is_correct: false,
            analysis: "符号错误".into(),
            created_at: chrono::Utc::now(),
            pinned: false,
            deleted_at: None,
        };
        let id1 = store.save(&m1).await.unwrap();
        let mut m2 = m1.clone();
        m2.id = MistakeId(uuid::Uuid::new_v4());
        let id2 = store.save(&m2).await.unwrap();

        store
            .update(
                &id1,
                &MistakePatch {
                    is_correct: Some(true),
                    pinned: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let got = store.get(&id1).await.unwrap().unwrap();
        assert!(got.is_correct);
        assert!(got.pinned);

        store.remove(&id2).await.unwrap();
        assert!(store.get(&id2).await.unwrap().is_none());
        assert_eq!(store.list(&MistakeFilter::default()).await.unwrap().len(), 1);

        let deleted = store.remove_many(&[id1, id2]).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(store.list(&MistakeFilter::default()).await.unwrap().is_empty());
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

        // assistant（模型）消息不可编辑。
        assert!(store.derive_branch(&key, a1.id, "回答一（修订）").await.is_err());

        // 编辑 user 消息：派生新分支，新文本生效（改完重发语义）。
        let path3 = store
            .derive_branch(&key, u1.id, "第一问（改错字）")
            .await
            .unwrap();
        assert_eq!(path3.len(), 1);
        assert!(
            matches!(&path3[0].kind, MessageKind::User { text, .. } if text == "第一问（改错字）")
        );
        // 历史保留：JSONL 共 5 条，活跃路径只剩 1 条。
        assert_eq!(store.read_all(&key).await.unwrap().len(), 5);
        assert_eq!(store.read_path(&key).await.unwrap().len(), 1);

        // 切回 a2 分支（4 条原始路径）。
        let path2 = store.switch_branch(&key, a2.id).await.unwrap();
        assert_eq!(path2.len(), 4);
        assert_eq!(path2[3].id, a2.id);
        assert_eq!(store.read_path(&key).await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn edit_user_message_keeps_attachments_and_clears_display_text() {
        let store = MemoryStorage::new();
        let key = SessionKey::new();
        store
            .create_session(&key, &SessionMeta::new(key))
            .await
            .unwrap();
        let u1 = Message {
            id: MessageId::new(),
            parent_id: None,
            kind: MessageKind::User {
                text: "帮我看看这道题\n附件：/tmp/mistake-agent-x|math.png".into(),
                display_text: Some("展示文本".into()),
                attachments: vec![crate::kernel::message::Attachment {
                    mime: "image/png".into(),
                    data_base64: "AAAA".into(),
                }],
            },
            created_at: chrono::Utc::now(),
        };
        store.append_message(&key, &u1).await.unwrap();

        let path = store
            .derive_branch(&key, u1.id, "帮我看看这道题（改错字）")
            .await
            .unwrap();
        assert_eq!(path.len(), 1);
        assert!(
            matches!(
                &path[0].kind,
                MessageKind::User {
                    text,
                    display_text,
                    attachments,
                } if text == "帮我看看这道题（改错字）"
                    && display_text.is_none()
                    && attachments.len() == 1
                    && attachments[0].data_base64 == "AAAA"
            )
        );
        // 只能编辑 user：assistant / system 均拒绝。
        let mut a1 = Message::assistant("回答");
        a1.parent_id = Some(path[0].id);
        store.append_message(&key, &a1).await.unwrap();
        assert!(store.derive_branch(&key, a1.id, "改").await.is_err());
        let mut sys = Message::system("系统提示");
        sys.parent_id = Some(path[0].id);
        store.append_message(&key, &sys).await.unwrap();
        assert!(store.derive_branch(&key, sys.id, "改").await.is_err());
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
