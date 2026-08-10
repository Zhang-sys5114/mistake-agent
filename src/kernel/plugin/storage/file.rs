//! storage 服务：M1 内存实现 + M2 文件持久化（会话 JSONL / 错题 JSON / 审计 JSONL 轮转）。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::kernel::agent::session::{Goal, SessionKey, SessionMeta, SessionStatus};
use crate::kernel::audit::{AuditRecord, AuditSink};
use crate::kernel::message::{Message, MessageId, MessageKind};
use crate::kernel::plugin::services::{
    Mistake, MistakeFilter, MistakeId, MistakePatch, MistakeStore, SessionStore, StorageError,
    StorageService,
};
use std::path::{Path, PathBuf};

use super::Inner;

use super::active_chain;

#[derive(Clone)]
pub struct FileStorage {
    root: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

impl FileStorage {
    pub fn open(root: &Path) -> Result<Self, StorageError> {
        let sessions_dir = root.join("sessions");
        let mistakes_dir = root.join("mistakes");
        // 子目录已由 bootstrap::init_data_root（Kernel::new 引导）创建，此处不再懒创建。

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
        let (messages, active_path) = {
            let inner = self.inner.lock().expect("storage poisoned");
            let messages = inner
                .messages
                .get(key)
                .cloned()
                .ok_or(StorageError::SessionNotFound(*key))?;
            let active_path = inner.sessions.get(key).and_then(|m| m.active_path);
            (messages, active_path)
        };
        Ok(active_chain(&messages, active_path))
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

    async fn set_active_path(
        &self,
        key: &SessionKey,
        message_id: Option<MessageId>,
    ) -> Result<(), StorageError> {
        self.inner
            .lock()
            .expect("storage poisoned")
            .sessions
            .get_mut(key)
            .ok_or(StorageError::SessionNotFound(*key))?
            .active_path = message_id;
        self.persist_session_meta(key)
    }

    async fn derive_branch(
        &self,
        key: &SessionKey,
        message_id: MessageId,
        text: &str,
    ) -> Result<Vec<Message>, StorageError> {
        let (messages, active_path) = {
            let inner = self.inner.lock().expect("storage poisoned");
            let messages = inner
                .messages
                .get(key)
                .cloned()
                .ok_or(StorageError::SessionNotFound(*key))?;
            let active_path = inner.sessions.get(key).and_then(|m| m.active_path);
            (messages, active_path)
        };
        let chain = active_chain(&messages, active_path);
        let idx = chain
            .iter()
            .position(|m| m.id == message_id)
            .ok_or(StorageError::Internal("消息不在活跃路径".into()))?;
        let original = &chain[idx];
        let mut new_msg = original.clone();
        new_msg.id = MessageId::new();
        new_msg.parent_id = original.parent_id;
        new_msg.created_at = chrono::Utc::now();
        match &mut new_msg.kind {
            // 仅允许编辑用户消息（改完重发）；assistant 等由模型生成，不可手改。
            MessageKind::User {
                text: t,
                display_text,
                ..
            } => {
                // 编辑用户消息：新文本作为模型指令与展示文本，附件保留（改完重发语义）。
                *t = text.to_string();
                *display_text = None;
            }
            _ => {
                return Err(StorageError::Internal("只能编辑 user 消息".into()));
            }
        };

        let mut new_path = chain[..idx].to_vec();
        new_path.push(new_msg.clone());
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let path = inner
                .messages
                .get_mut(key)
                .ok_or(StorageError::SessionNotFound(*key))?;
            path.push(new_msg);
            let meta = inner
                .sessions
                .get_mut(key)
                .ok_or(StorageError::SessionNotFound(*key))?;
            meta.active_path = new_path.last().map(|m| m.id);
        }
        self.persist_session_meta(key)?;
        Ok(new_path)
    }

    async fn switch_branch(
        &self,
        key: &SessionKey,
        message_id: MessageId,
    ) -> Result<Vec<Message>, StorageError> {
        let messages = self.read_all(key).await?;
        if !messages.iter().any(|m| m.id == message_id) {
            return Err(StorageError::Internal("消息不存在".into()));
        }
        let chain = active_chain(&messages, Some(message_id));
        self.set_active_path(key, Some(message_id)).await?;
        Ok(chain)
    }

    async fn splice_compaction(
        &self,
        key: &SessionKey,
        summary: &Message,
        tail_start: MessageId,
    ) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let path = inner
                .messages
                .get_mut(key)
                .ok_or(StorageError::SessionNotFound(*key))?;
            let tail = path
                .iter_mut()
                .find(|m| m.id == tail_start)
                .ok_or(StorageError::Internal("保留段首条不存在".into()))?;
            tail.parent_id = Some(summary.id);
            path.push(summary.clone());
        }
        self.persist_session_meta(key)
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
            .find(|m| m.id == *id && m.deleted_at.is_none())
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
                    && m.deleted_at.is_none()
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
            if m.deleted_at.is_some() {
                return Err(StorageError::MistakeNotFound(id.to_string()));
            }
            if let Some(s) = &patch.subject {
                m.subject = s.clone();
            }
            if let Some(k) = &patch.knowledge_point {
                m.knowledge_point = k.clone();
            }
            if let Some(q) = &patch.question {
                m.question = q.clone();
            }
            if let Some(s) = &patch.student_answer {
                m.student_answer = s.clone();
            }
            if let Some(r) = &patch.reference_answer {
                m.reference_answer = r.clone();
            }
            if let Some(a) = &patch.analysis {
                m.analysis = a.clone();
            }
            if let Some(c) = patch.is_correct {
                m.is_correct = c;
            }
            if let Some(p) = patch.pinned {
                m.pinned = p;
            }
        }
        self.persist_mistakes()
    }

    async fn remove(&self, id: &MistakeId) -> Result<(), StorageError> {
        {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let m = inner
                .mistakes
                .iter_mut()
                .find(|m| m.id == *id)
                .ok_or(StorageError::MistakeNotFound(id.to_string()))?;
            if m.deleted_at.is_none() {
                m.deleted_at = Some(chrono::Utc::now());
            }
        }
        self.persist_mistakes()
    }

    async fn remove_many(&self, ids: &[MistakeId]) -> Result<usize, StorageError> {
        let deleted = {
            let mut inner = self.inner.lock().expect("storage poisoned");
            let now = chrono::Utc::now();
            let mut deleted = 0usize;
            for id in ids {
                if let Some(m) = inner.mistakes.iter_mut().find(|m| m.id == *id)
                    && m.deleted_at.is_none()
                {
                    m.deleted_at = Some(now);
                    deleted += 1;
                }
            }
            deleted
        };
        self.persist_mistakes()?;
        Ok(deleted)
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

// ---------- 域内文件 IO（ADR-0042） ----------

use crate::kernel::plugin::services::{Domain, DomainIo, RelPath, TmpIo};

impl FileStorage {
    fn domain_root(&self, domain: Domain) -> PathBuf {
        self.root.join(domain.as_dir())
    }

    /// 域内路径解析：RelPath 已过白名单（类型上无遍历），拼接后 canonicalize 兜底
    /// 防符号链接逃逸——校验结果必须落在域根内（带分隔符边界，防前缀假匹配）。
    fn resolve_in_domain(&self, domain: Domain, rel: &RelPath) -> Result<PathBuf, StorageError> {
        let root = self.domain_root(domain);
        let mut path = root.clone();
        for seg in rel.segments() {
            path.push(seg);
        }
        let root_canon = dunce::canonicalize(&root)
            .map_err(|e| StorageError::Io(format!("域根不可用 {root:?}：{e}")))?;
        let path_canon = dunce::canonicalize(&path)
            .map_err(|e| StorageError::Io(format!("路径不可达 {path:?}：{e}")))?;
        let boundary = format!("{}{}", root_canon.to_string_lossy(), std::path::MAIN_SEPARATOR);
        let ok = path_canon.starts_with(&root_canon)
            && path_canon.to_string_lossy().starts_with(&boundary);
        if !ok {
            return Err(StorageError::InvalidPath(format!(
                "路径越出域根：{path:?}"
            )));
        }
        Ok(path_canon)
    }

    /// 历史路径解析（仅迁移用，ADR-0042）：允许非 ASCII 段（旧存储布局），
    /// 但拒绝 `..`/`\`/绝对路径/空段——宽松不等于不校验，遍历向量仍被挡，
    /// canonicalize + 域根边界兜底与 RelPath 路径一致。
    fn resolve_legacy_in_domain(&self, domain: Domain, legacy_rel: &str) -> Result<PathBuf, StorageError> {
        if legacy_rel.is_empty() {
            return Err(StorageError::InvalidPath("路径为空".into()));
        }
        if legacy_rel.starts_with('/') || legacy_rel.contains('\\') || legacy_rel.contains(':') {
            return Err(StorageError::InvalidPath(format!(
                "历史路径含非法字符：{legacy_rel}"
            )));
        }
        for seg in legacy_rel.split('/') {
            if seg.is_empty() || seg == "." || seg == ".." {
                return Err(StorageError::InvalidPath(format!(
                    "历史路径含非法段：{seg}"
                )));
            }
        }
        let root = self.domain_root(domain);
        let path = root.join(legacy_rel);
        let root_canon = dunce::canonicalize(&root)
            .map_err(|e| StorageError::Io(format!("域根不可用 {root:?}：{e}")))?;
        let path_canon = dunce::canonicalize(&path)
            .map_err(|e| StorageError::Io(format!("路径不可达 {path:?}：{e}")))?;
        let boundary = format!("{}{}", root_canon.to_string_lossy(), std::path::MAIN_SEPARATOR);
        if !path_canon.starts_with(&root_canon)
            || !path_canon.to_string_lossy().starts_with(&boundary)
        {
            return Err(StorageError::InvalidPath(format!(
                "路径越出域根：{path:?}"
            )));
        }
        Ok(path_canon)
    }
}

#[async_trait]
impl DomainIo for FileStorage {
    async fn read(&self, domain: Domain, rel: &RelPath) -> Result<Vec<u8>, StorageError> {
        let path = self.resolve_in_domain(domain, rel)?;
        let bytes = std::fs::read(&path)
            .map_err(|e| StorageError::Io(format!("读取失败 {path:?}：{e}")))?;
        self.append(AuditRecord::FileIo {
            domain: domain.as_dir().into(),
            rel: rel.as_str(),
            op: "read".into(),
        });
        Ok(bytes)
    }

    async fn write(&self, domain: Domain, rel: &RelPath, bytes: &[u8]) -> Result<(), StorageError> {
        let root = self.domain_root(domain);
        let mut path = root.clone();
        for seg in rel.segments() {
            path.push(seg);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::Io(format!("创建目录失败 {parent:?}：{e}")))?;
        }
        // 原子写：tmp + rename（不 follow 符号链接）。
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)
            .map_err(|e| StorageError::Io(format!("写临时文件失败 {tmp:?}：{e}")))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| StorageError::Io(format!("原子改名失败 {path:?}：{e}")))?;
        self.append(AuditRecord::FileIo {
            domain: domain.as_dir().into(),
            rel: rel.as_str(),
            op: "write".into(),
        });
        Ok(())
    }

    async fn remove(&self, domain: Domain, rel: &RelPath) -> Result<(), StorageError> {
        let path = self.resolve_in_domain(domain, rel)?;
        std::fs::remove_file(&path)
            .map_err(|e| StorageError::Io(format!("删除失败 {path:?}：{e}")))?;
        self.append(AuditRecord::FileIo {
            domain: domain.as_dir().into(),
            rel: rel.as_str(),
            op: "remove".into(),
        });
        Ok(())
    }

    async fn remove_tree(&self, domain: Domain, rel: &RelPath) -> Result<(), StorageError> {
        let root = self.domain_root(domain);
        let mut path = root.clone();
        for seg in rel.segments() {
            path.push(seg);
        }
        let root_canon = dunce::canonicalize(&root)
            .map_err(|e| StorageError::Io(format!("域根不可用 {root:?}：{e}")))?;
        let path_canon = dunce::canonicalize(&path)
            .map_err(|e| StorageError::Io(format!("路径不可达 {path:?}：{e}")))?;
        // 子树删除绝不允许指向域根本身。
        if path_canon == root_canon {
            return Err(StorageError::InvalidPath("拒绝删除域根".into()));
        }
        let boundary = format!("{}{}", root_canon.to_string_lossy(), std::path::MAIN_SEPARATOR);
        if !path_canon.to_string_lossy().starts_with(&boundary) {
            return Err(StorageError::InvalidPath(format!(
                "路径越出域根：{path:?}"
            )));
        }
        std::fs::remove_dir_all(&path_canon)
            .map_err(|e| StorageError::Io(format!("删除子树失败 {path_canon:?}：{e}")))?;
        self.append(AuditRecord::FileIo {
            domain: domain.as_dir().into(),
            rel: rel.as_str(),
            op: "remove_tree".into(),
        });
        Ok(())
    }

    async fn list(&self, domain: Domain) -> Result<Vec<String>, StorageError> {
        let root = self.domain_root(domain);
        let mut out = Vec::new();
        Self::walk_domain(&root, &root, &mut out)?;
        out.sort();
        Ok(out)
    }

    async fn read_legacy(&self, domain: Domain, legacy_rel: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.resolve_legacy_in_domain(domain, legacy_rel)?;
        let bytes = std::fs::read(&path)
            .map_err(|e| StorageError::Io(format!("读取失败 {path:?}：{e}")))?;
        self.append(AuditRecord::FileIo {
            domain: domain.as_dir().into(),
            rel: legacy_rel.into(),
            op: "read_legacy".into(),
        });
        Ok(bytes)
    }

    async fn remove_legacy(&self, domain: Domain, legacy_rel: &str) -> Result<(), StorageError> {
        let path = self.resolve_legacy_in_domain(domain, legacy_rel)?;
        std::fs::remove_file(&path)
            .map_err(|e| StorageError::Io(format!("删除失败 {path:?}：{e}")))?;
        self.append(AuditRecord::FileIo {
            domain: domain.as_dir().into(),
            rel: legacy_rel.into(),
            op: "remove_legacy".into(),
        });
        Ok(())
    }
}

impl FileStorage {
    fn walk_domain(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), StorageError> {
        for entry in std::fs::read_dir(dir)
            .map_err(|e| StorageError::Io(format!("读取目录失败 {dir:?}：{e}")))?
        {
            let entry = entry.map_err(|e| StorageError::Io(format!("读取目录项失败：{e}")))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| StorageError::Io(format!("读取文件类型失败：{e}")))?;
            if file_type.is_dir() {
                Self::walk_domain(root, &path, out)?;
            } else if file_type.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|_| StorageError::Io("路径越出域根".into()))?;
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }
}

// ---------- 系统 temp 暂存文件 IO（ADR-0042） ----------

impl FileStorage {
    /// 暂存白名单：canonicalize（防符号链接逃逸）必须落在系统 temp 目录内
    /// 且文件名带 `mistake-agent-` 前缀。Windows 上 canonicalize 的 `\\?\` verbatim
    /// 前缀由 dunce 统一规范化后再比较（两侧一致）。
    fn staged_path_allowed(&self, path: &Path) -> Result<PathBuf, StorageError> {
        let Ok(canonical) = dunce::canonicalize(path) else {
            return Err(StorageError::InvalidPath(format!(
                "路径不可达：{path:?}"
            )));
        };
        let Some(name) = canonical.file_name().and_then(|n| n.to_str()) else {
            return Err(StorageError::InvalidPath("暂存文件名非法".into()));
        };
        if !name.starts_with("mistake-agent-") {
            return Err(StorageError::InvalidPath(format!(
                "暂存文件必须带 mistake-agent- 前缀：{name}"
            )));
        }
        let temp = std::env::temp_dir();
        let Ok(temp_canonical) = dunce::canonicalize(&temp) else {
            return Err(StorageError::Io("系统临时目录不可用".into()));
        };
        if !canonical.starts_with(&temp_canonical) {
            return Err(StorageError::InvalidPath(format!(
                "暂存文件越出系统临时目录：{path:?}"
            )));
        }
        Ok(canonical)
    }
}

#[async_trait]
impl TmpIo for FileStorage {
    async fn read_staged(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let canonical = self.staged_path_allowed(Path::new(path))?;
        let bytes = std::fs::read(&canonical)
            .map_err(|e| StorageError::Io(format!("读取暂存失败 {canonical:?}：{e}")))?;
        self.append(AuditRecord::StagedFileIo {
            path: canonical.to_string_lossy().into_owned(),
            op: "read".into(),
        });
        Ok(bytes)
    }

    async fn remove_staged(&self, path: &str) -> Result<(), StorageError> {
        let canonical = self.staged_path_allowed(Path::new(path))?;
        std::fs::remove_file(&canonical)
            .map_err(|e| StorageError::Io(format!("删除暂存失败 {canonical:?}：{e}")))?;
        self.append(AuditRecord::StagedFileIo {
            path: canonical.to_string_lossy().into_owned(),
            op: "remove".into(),
        });
        Ok(())
    }
}

/// 统一存储后端：优先文件持久化，打不开时回退内存（保证应用可启动）。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::plugin::services::{Domain, DomainIo, RelPath, StorageHandle, TmpIo};
    use std::sync::Arc;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mistake-agent-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::create_dir_all(dir.join("memory")).unwrap();
        dir
    }

    fn real_store(dir: &std::path::Path) -> Arc<FileStorage> {
        Arc::new(FileStorage::open(dir).unwrap())
    }

    // ---------- RelPath：白名单构造即校验（ADR-0042） ----------

    #[test]
    fn relpath_accepts_whitelist_segments() {
        for ok in ["gaokao_pool.json", "a..md", "a/b/c.md", "dir_1-2/file.txt", "pool.json"] {
            assert!(RelPath::parse(ok).is_ok(), "应接受：{ok}");
        }
    }

    #[test]
    fn relpath_rejects_traversal_vectors() {
        for bad in [
            "..", "../", "../x", "a/../b", "a/..", "....//", ".", "./a", "a/.", "a..", "..a",
            "a//b", "/abs", "a\\b", "a:b", "a b", "a\u{0000}b", "a\tb", "。", "a。b",
        ] {
            assert!(RelPath::parse(bad).is_err(), "应拒绝：{bad:?}");
        }
    }

    #[test]
    fn relpath_rejects_leading_trailing_dots() {
        // 尾点（Windows 别名陷阱）与首点（隐藏文件/…）都拒绝。
        for bad in ["a.", ".a", "...", "....", ".hidden"] {
            assert!(RelPath::parse(bad).is_err(), "应拒绝：{bad:?}");
        }
    }

    #[test]
    fn relpath_rejects_empty_and_unicode() {
        assert!(RelPath::parse("").is_err());
        assert!(RelPath::parse("数据/文件").is_err(), "非 ASCII 应拒绝（同形字符攻击面）");
    }

    // ---------- DomainIo：域逃逸（ADR-0042） ----------

    #[tokio::test]
    async fn domain_write_read_roundtrip_and_list() {
        let dir = temp_root("domain-roundtrip");
        let store = real_store(&dir);
        let rel = RelPath::parse("gaokao_pool.json").unwrap();
        DomainIo::write(store.as_ref(), Domain::Data, &rel, b"[]").await.unwrap();
        assert_eq!(DomainIo::read(store.as_ref(), Domain::Data, &rel).await.unwrap(), b"[]");
        let listed = DomainIo::list(store.as_ref(), Domain::Data).await.unwrap();
        assert_eq!(listed, vec!["gaokao_pool.json".to_string()]);
        DomainIo::remove(store.as_ref(), Domain::Data, &rel).await.unwrap();
        assert!(DomainIo::list(store.as_ref(), Domain::Data).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn domain_rejects_removal_of_domain_root() {
        let dir = temp_root("domain-root");
        let store = real_store(&dir);
        // RelPath 至少一段，无法表达"域根"；验证子树删除只删子树、不越出域根。
        let nested = RelPath::parse("sub/nested/file.md").unwrap();
        DomainIo::write(store.as_ref(), Domain::Data, &nested, b"x").await.unwrap();
        let sub = RelPath::parse("sub").unwrap();
        DomainIo::remove_tree(store.as_ref(), Domain::Data, &sub).await.unwrap();
        assert!(DomainIo::list(store.as_ref(), Domain::Data).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn domain_rejects_escape_outside_root() {
        let dir = temp_root("domain-escape");
        let store = real_store(&dir);
        // 域根外真实存在一个文件：直接经 trait 逃逸路径不可达（RelPath 无 `..`）。
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, b"secret").unwrap();
        let rel = RelPath::parse("x.txt").unwrap();
        // write 会写到域内而非外面；断言外部文件未被触碰。
        DomainIo::write(store.as_ref(), Domain::Data, &rel, b"y").await.unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- TmpIo：系统 temp 白名单（ADR-0042） ----------

    #[tokio::test]
    async fn tmp_reads_only_staged_prefix_in_temp() {
        let dir = temp_root("tmp-whitelist");
        let store = real_store(&dir);
        let temp = std::env::temp_dir();
        let ok = temp.join(format!("mistake-agent-{}.png", uuid::Uuid::new_v4()));
        let no_prefix = temp.join(format!("other-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&ok, b"img").unwrap();
        std::fs::write(&no_prefix, b"img").unwrap();
        assert_eq!(TmpIo::read_staged(store.as_ref(), &ok.to_string_lossy()).await.unwrap(), b"img");
        assert!(TmpIo::read_staged(store.as_ref(), &no_prefix.to_string_lossy()).await.is_err(), "非前缀应拒绝");
        assert!(TmpIo::read_staged(store.as_ref(), "/etc/passwd").await.is_err(), "系统路径应拒绝");
        // 删除：只删白名单内的。
        TmpIo::remove_staged(store.as_ref(), &ok.to_string_lossy()).await.unwrap();
        assert!(!ok.exists());
        assert!(no_prefix.exists(), "非前缀文件不受影响");
        let _ = std::fs::remove_file(&no_prefix);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn storage_handle_semantic_methods_route_to_io() {
        let dir = temp_root("handle-semantics");
        let store = real_store(&dir);
        let handle = StorageHandle::new(store.clone()).with_io(store.clone(), store.clone());
        handle.write_data_file("pool.json", r#"[]"#).await.unwrap();
        assert_eq!(handle.read_data_file("pool.json").await.unwrap(), "[]");
        assert!(handle.read_data_file("a/../b").await.is_err(), "句柄层同样拒绝遍历");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn memory_domain_isolation_from_data() {
        let dir = temp_root("domain-isolation");
        let store = real_store(&dir);
        let mem_rel = RelPath::parse("math.md").unwrap();
        let data_rel = RelPath::parse("math.md").unwrap();
        DomainIo::write(store.as_ref(), Domain::Memory, &mem_rel, b"m").await.unwrap();
        DomainIo::write(store.as_ref(), Domain::Data, &data_rel, b"d").await.unwrap();
        assert_eq!(
            DomainIo::read(store.as_ref(), Domain::Memory, &mem_rel)
                .await
                .unwrap(),
            b"m"
        );
        assert_eq!(
            DomainIo::read(store.as_ref(), Domain::Data, &data_rel)
                .await
                .unwrap(),
            b"d"
        );
        // 同名不同域互不干扰。
        assert_eq!(
            DomainIo::list(store.as_ref(), Domain::Memory).await.unwrap(),
            vec!["math.md".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
