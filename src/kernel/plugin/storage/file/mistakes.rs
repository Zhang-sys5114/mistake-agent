use super::*;


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
