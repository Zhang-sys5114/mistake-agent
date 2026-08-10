//! 内存版记忆（测试/回退实现）。

use super::*;

#[derive(Default, Clone)]
pub struct InMemoryMemory {
    entries: Arc<Mutex<BTreeMap<String, String>>>,
}

impl InMemoryMemory {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MemoryService for InMemoryMemory {
    async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError> {
        self.entries
            .lock()
            .expect("memory poisoned")
            .insert(path.as_str(), content.to_string());
        Ok(())
    }

    async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
        let entries = self.entries.lock().expect("memory poisoned");
        match path {
            None => Ok(MemoryView::Listing(entries.keys().cloned().collect())),
            Some(p) => {
                let key = p.as_str();
                let content = entries
                    .get(&key)
                    .ok_or_else(|| MemoryError::NotFound(key.clone()))?
                    .clone();
                Ok(MemoryView::Entry {
                    path: p.clone(),
                    content,
                })
            }
        }
    }

    async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError> {
        let prefix = format!("{}/", path.as_str());
        let mut entries = self.entries.lock().expect("memory poisoned");
        let keys: Vec<String> = entries
            .keys()
            .filter(|k| *k == &path.as_str() || k.starts_with(&prefix))
            .cloned()
            .collect();
        if keys.is_empty() {
            return Err(MemoryError::NotFound(path.as_str()));
        }
        for k in keys {
            entries.remove(&k);
        }
        Ok(())
    }
}
