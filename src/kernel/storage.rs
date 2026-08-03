//! storage 服务：M1 内存实现 + M2 文件持久化（会话 JSONL / 错题 JSON / 审计 JSONL 轮转）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::kernel::audit::{AuditRecord, AuditSink};
use crate::kernel::message::{Message, MessageId};
use crate::kernel::services::{
    Mistake, MistakeFilter, MistakeId, MistakePatch, MistakeStore, SessionStore, StorageError,
    StorageService,
};
use crate::kernel::session::{Goal, SessionKey, SessionMeta, SessionStatus};

#[derive(Default)]
struct Inner {
    sessions: HashMap<SessionKey, SessionMeta>,
    messages: HashMap<SessionKey, Vec<Message>>,
    mistakes: Vec<Mistake>,
    audit: Vec<AuditRecord>,
}

/// M1 内存 storage（M2 换成 JSONL + 错题本 JSON）。
#[derive(Clone)]
pub struct MemoryStorage {
    inner: Arc<Mutex<Inner>>,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }
}

#[async_trait]
impl SessionStore for MemoryStorage {
    async fn create_session(
        &self,
        key: &SessionKey,
        meta: &SessionMeta,
    ) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        if inner.sessions.contains_key(key) {
            return Err(StorageError::AlreadyExists(key.to_string()));
        }
        inner.sessions.insert(*key, meta.clone());
        inner.messages.insert(*key, Vec::new());
        Ok(())
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionMeta>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .get(key)
            .cloned())
    }

    async fn append_message(&self, key: &SessionKey, msg: &Message) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        let path = inner
            .messages
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?;
        path.push(msg.clone());
        Ok(())
    }

    async fn read_path(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError> {
        self.read_all(key).await
    }

    async fn read_all(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .messages
            .get(key)
            .cloned()
            .unwrap_or_default())
    }

    async fn set_goal(&self, key: &SessionKey, goal: &Goal) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        let meta = inner
            .sessions
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?;
        meta.goal = Some(goal.clone());
        Ok(())
    }

    async fn archive(&self, key: &SessionKey) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        let meta = inner
            .sessions
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?;
        meta.status = SessionStatus::Archived;
        meta.archived_at = Some(chrono::Utc::now());
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<SessionMeta>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .values()
            .cloned()
            .collect())
    }

    async fn set_last_activity(
        &self,
        key: &SessionKey,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        let meta = inner
            .sessions
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?;
        meta.last_activity_at = at;
        Ok(())
    }
}

#[async_trait]
impl MistakeStore for MemoryStorage {
    async fn save(&self, mistake: &Mistake) -> Result<MistakeId, StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        if inner.mistakes.iter().any(|m| m.id == mistake.id) {
            return Err(StorageError::AlreadyExists(mistake.id.to_string()));
        }
        inner.mistakes.push(mistake.clone());
        Ok(mistake.id)
    }

    async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .mistakes
            .iter()
            .find(|m| m.id == *id)
            .cloned())
    }

    async fn list(&self, filter: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
        let inner = self.inner.lock().expect("storage poisoned");
        Ok(inner
            .mistakes
            .iter()
            .filter(|m| {
                filter
                    .subject
                    .as_deref()
                    .map(|s| m.subject == s)
                    .unwrap_or(true)
                    && filter
                        .knowledge_point
                        .as_deref()
                        .map(|k| m.knowledge_point == k)
                        .unwrap_or(true)
                    && filter.is_correct.map(|c| m.is_correct == c).unwrap_or(true)
            })
            .cloned()
            .collect())
    }

    async fn update(&self, id: &MistakeId, patch: &MistakePatch) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        let m = inner
            .mistakes
            .iter_mut()
            .find(|m| m.id == *id)
            .ok_or(StorageError::MistakeNotFound(id.to_string()))?;
        if let Some(k) = &patch.knowledge_point {
            m.knowledge_point = k.clone();
        }
        if let Some(a) = &patch.analysis {
            m.analysis = a.clone();
        }
        if let Some(c) = patch.is_correct {
            m.is_correct = c;
        }
        Ok(())
    }

    async fn remove(&self, id: &MistakeId) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("storage poisoned");
        let before = inner.mistakes.len();
        inner.mistakes.retain(|m| m.id != *id);
        if inner.mistakes.len() == before {
            return Err(StorageError::MistakeNotFound(id.to_string()));
        }
        Ok(())
    }
}

impl AuditSink for MemoryStorage {
    fn append(&self, record: AuditRecord) {
        self.inner
            .lock()
            .expect("storage poisoned")
            .audit
            .push(record);
    }
}

impl StorageService for MemoryStorage {}

// ---------- 文件持久化实现（M2） ----------

use std::path::{Path, PathBuf};

/// 文件版 storage：数据根目录布局（ADR-0011）：
/// - sessions/<key>.jsonl：首行 SessionMeta，随后每行一条 Message
/// - mistakes/mistakes.json：错题数组（原子写：临时文件 + rename）
/// - audit/audit.jsonl：追加式，10MB 归档轮转
#[derive(Clone)]
pub struct FileStorage {
    root: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

impl FileStorage {
    pub fn open(root: &Path) -> Result<Self, StorageError> {
        let sessions_dir = root.join("sessions");
        let mistakes_dir = root.join("mistakes");
        let audit_dir = root.join("audit");
        for dir in [&sessions_dir, &mistakes_dir, &audit_dir] {
            std::fs::create_dir_all(dir)
                .map_err(|e| StorageError::Io(format!("创建目录失败 {dir:?}：{e}")))?;
        }

        let inner = Arc::new(Mutex::new(Inner::default()));
        // 加载错题本。
        let mistakes_file = mistakes_dir.join("mistakes.json");
        if let Ok(text) = std::fs::read_to_string(&mistakes_file) {
            let loaded: Vec<Mistake> = serde_json::from_str(&text)
                .map_err(|e| StorageError::Corrupt(format!("错题本解析失败：{e}")))?;
            inner.lock().expect("storage poisoned").mistakes = loaded;
        }
        // 加载会话（每个 <key>.jsonl 首行为元数据）。
        let mut loaded_metas = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Some((meta_line, _rest)) = text.split_once('\n') else {
                    continue;
                };
                if let Ok(meta) = serde_json::from_str::<SessionMeta>(meta_line) {
                    let key = meta.key;
                    let messages: Vec<Message> = text
                        .lines()
                        .skip(1)
                        .filter(|l| !l.trim().is_empty())
                        .filter_map(|l| serde_json::from_str(l).ok())
                        .collect();
                    let mut inner = inner.lock().expect("storage poisoned");
                    inner.sessions.insert(key, meta);
                    inner.messages.insert(key, messages);
                    loaded_metas.push(key);
                }
            }
        }
        let _ = loaded_metas;
        Ok(Self {
            root: root.to_path_buf(),
            inner,
        })
    }

    fn session_path(&self, key: &SessionKey) -> PathBuf {
        self.root.join("sessions").join(format!("{key}.jsonl"))
    }

    fn mistakes_path(&self) -> PathBuf {
        self.root.join("mistakes").join("mistakes.json")
    }

    fn audit_path(&self) -> PathBuf {
        self.root.join("audit").join("audit.jsonl")
    }

    fn persist_mistakes(&self) -> Result<(), StorageError> {
        let mistakes = self
            .inner
            .lock()
            .expect("storage poisoned")
            .mistakes
            .clone();
        atomic_write_json(&self.mistakes_path(), &mistakes)
    }

    fn persist_session_meta(&self, key: &SessionKey) -> Result<(), StorageError> {
        let (meta, messages) = {
            let inner = self.inner.lock().expect("storage poisoned");
            let meta = inner
                .sessions
                .get(key)
                .ok_or(StorageError::SessionNotFound(*key))?
                .clone();
            let messages = inner.messages.get(key).cloned().unwrap_or_default();
            (meta, messages)
        };
        let path = self.session_path(key);
        let mut out = String::new();
        out.push_str(
            &serde_json::to_string(&meta)
                .map_err(|e| StorageError::Io(format!("会话元数据序列化失败：{e}")))?,
        );
        out.push('\n');
        for msg in &messages {
            out.push_str(
                &serde_json::to_string(msg)
                    .map_err(|e| StorageError::Io(format!("消息序列化失败：{e}")))?,
            );
            out.push('\n');
        }
        atomic_write_str(&path, &out)
    }
}

fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| StorageError::Io(format!("序列化失败：{e}")))?;
    atomic_write_str(path, &text)
}

fn atomic_write_str(path: &Path, text: &str) -> Result<(), StorageError> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| StorageError::Io(format!("写临时文件失败：{e}")))?;
    std::fs::rename(&tmp, path).map_err(|e| StorageError::Io(format!("原子改名失败：{e}")))?;
    Ok(())
}

#[async_trait]
impl SessionStore for FileStorage {
    async fn create_session(
        &self,
        key: &SessionKey,
        meta: &SessionMeta,
    ) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            if inner.sessions.contains_key(key) {
                return Err(StorageError::AlreadyExists(key.to_string()));
            }
            inner.sessions.insert(*key, meta.clone());
            inner.messages.insert(*key, Vec::new());
        }
        self.persist_session_meta(key)
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionMeta>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .get(key)
            .cloned())
    }

    async fn append_message(&self, key: &SessionKey, msg: &Message) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let path = inner
                .messages
                .get_mut(key)
                .ok_or(StorageError::SessionNotFound(*key))?;
            path.push(msg.clone());
        }
        let path = self.session_path(key);
        let mut line = serde_json::to_string(msg)
            .map_err(|e| StorageError::Io(format!("消息序列化失败：{e}")))?;
        line.push('\n');
        append_line(&path, &line)
    }

    async fn read_path(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError> {
        self.read_all(key).await
    }

    async fn read_all(&self, key: &SessionKey) -> Result<Vec<Message>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .messages
            .get(key)
            .cloned()
            .unwrap_or_default())
    }

    async fn set_goal(&self, key: &SessionKey, goal: &Goal) -> Result<(), StorageError> {
        self.inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?
            .goal = Some(goal.clone());
        self.persist_session_meta(key)
    }

    async fn archive(&self, key: &SessionKey) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let meta = inner
                .sessions
                .get_mut(key)
                .ok_or(StorageError::SessionNotFound(*key))?;
            meta.status = SessionStatus::Archived;
            meta.archived_at = Some(chrono::Utc::now());
        }
        self.persist_session_meta(key)
    }

    async fn list_sessions(&self) -> Result<Vec<SessionMeta>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .values()
            .cloned()
            .collect())
    }

    async fn set_last_activity(
        &self,
        key: &SessionKey,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StorageError> {
        self.inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?
            .last_activity_at = at;
        self.persist_session_meta(key)
    }
}

fn append_line(path: &Path, line: &str) -> Result<(), StorageError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| StorageError::Io(format!("打开失败 {path:?}：{e}")))?;
    file.write_all(line.as_bytes())
        .map_err(|e| StorageError::Io(format!("追加失败：{e}")))?;
    Ok(())
}

#[async_trait]
impl MistakeStore for FileStorage {
    async fn save(&self, mistake: &Mistake) -> Result<MistakeId, StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            if inner.mistakes.iter().any(|m| m.id == mistake.id) {
                return Err(StorageError::AlreadyExists(mistake.id.to_string()));
            }
            inner.mistakes.push(mistake.clone());
        }
        self.persist_mistakes()?;
        Ok(mistake.id)
    }

    async fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, StorageError> {
        Ok(self
            .inner
            .lock()
            .expect("storage poisoned")
            .mistakes
            .iter()
            .find(|m| m.id == *id)
            .cloned())
    }

    async fn list(&self, filter: &MistakeFilter) -> Result<Vec<Mistake>, StorageError> {
        let inner = self.inner.lock().expect("storage poisoned");
        Ok(inner
            .mistakes
            .iter()
            .filter(|m| {
                filter
                    .subject
                    .as_deref()
                    .map(|s| m.subject == s)
                    .unwrap_or(true)
                    && filter
                        .knowledge_point
                        .as_deref()
                        .map(|k| m.knowledge_point == k)
                        .unwrap_or(true)
                    && filter.is_correct.map(|c| m.is_correct == c).unwrap_or(true)
            })
            .cloned()
            .collect())
    }

    async fn update(&self, id: &MistakeId, patch: &MistakePatch) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let m = inner
                .mistakes
                .iter_mut()
                .find(|m| m.id == *id)
                .ok_or(StorageError::MistakeNotFound(id.to_string()))?;
            if let Some(k) = &patch.knowledge_point {
                m.knowledge_point = k.clone();
            }
            if let Some(a) = &patch.analysis {
                m.analysis = a.clone();
            }
            if let Some(c) = patch.is_correct {
                m.is_correct = c;
            }
        }
        self.persist_mistakes()
    }

    async fn remove(&self, id: &MistakeId) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let before = inner.mistakes.len();
            inner.mistakes.retain(|m| m.id != *id);
            if inner.mistakes.len() == before {
                return Err(StorageError::MistakeNotFound(id.to_string()));
            }
        }
        self.persist_mistakes()
    }
}

impl AuditSink for FileStorage {
    fn append(&self, record: AuditRecord) {
        self.inner
            .lock()
            .expect("storage poisoned")
            .audit
            .push(record.clone());
        if let Ok(line) = serde_json::to_string(&record) {
            let path = self.audit_path();
            let _ = append_line(&path, &format!("{line}\n"));
            rotate_if_large(&path);
        }
    }
}

impl StorageService for FileStorage {}

/// 统一存储后端：优先文件持久化，打不开时回退内存（保证应用可启动）。
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

/// 10MB 归档轮转（ADR-0017），保留 5 份。
fn rotate_if_large(path: &Path) {
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_BYTES {
        return;
    }
    for i in (1..5).rev() {
        let from = path.with_extension(format!("jsonl.{i}"));
        let to = path.with_extension(format!("jsonl.{}", i + 1));
        let _ = std::fs::rename(&from, &to);
    }
    let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    let _ = std::fs::File::create(path);
}

pub fn active_session(metas: &[SessionMeta]) -> Option<SessionMeta> {
    metas
        .iter()
        .find(|m| m.status == SessionStatus::Active)
        .cloned()
}

pub fn last_message_id(messages: &[Message]) -> Option<MessageId> {
    messages.last().map(|m| m.id)
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
}
