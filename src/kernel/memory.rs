//! memory 服务：M1 内存实现（目录式浏览，Q9 契约）。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::kernel::services::{MemoryError, MemoryPath, MemoryService, MemoryView};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_listing_and_subtree_remove() {
        let mem = InMemoryMemory::new();
        let p1 = MemoryPath::parse("数学/函数/二次函数").unwrap();
        let p2 = MemoryPath::parse("数学/几何").unwrap();
        mem.save(&p1, "顶点公式").await.unwrap();
        mem.save(&p2, "勾股定理").await.unwrap();

        let listing = mem.show(None).await.unwrap();
        assert_eq!(
            listing,
            MemoryView::Listing(vec![
                "数学/几何".to_string(),
                "数学/函数/二次函数".to_string(),
            ])
        );

        mem.remove(&MemoryPath::parse("数学").unwrap())
            .await
            .unwrap();
        let listing = mem.show(None).await.unwrap();
        assert_eq!(listing, MemoryView::Listing(vec![]));
    }

    #[test]
    fn path_rejects_escapes() {
        assert!(MemoryPath::parse("数学/../秘密").is_err());
        assert!(MemoryPath::parse("/绝对路径").is_err());
        assert!(MemoryPath::parse("数学//函数").is_err());
        assert!(MemoryPath::parse("数学/函数/二次函数").is_ok());
    }
}
