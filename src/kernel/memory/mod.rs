//! memory 服务（ADR-0008）：目录式记忆路由。
//! 内核插件信息：ServiceId::Memory；能力 = save/show/remove（路径校验、子树删除）；
//! 生产实现 = 文件持久化（数据根 memory/），InMemoryMemory 保留供测试。
//! - `InMemoryMemory`：测试/内存回退实现；
//! - `FileMemoryService`：数据根目录 memory/ 下的文件持久化（M2 生产实现）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::kernel::services::{MemoryError, MemoryPath, MemoryService, MemoryView};
use crate::kernel::settings::Settings;

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

/// 文件版记忆服务：条目落盘为 `memory/<路径>.md`。
///
/// 目录语义：
/// - `show(None)` = 递归列出全部条目（不带 `.md` 后缀，按路径排序）；
/// - `show(Some(p))` = 读取单个条目；
/// - `remove(p)` = 删除该条目及其子树（路径前缀匹配，`数学` 会删掉 `数学/…` 全部）。
pub struct FileMemoryService {
    root: PathBuf,
}

impl FileMemoryService {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn open_default() -> Result<Self, MemoryError> {
        let root = Settings::data_root().join("memory");
        std::fs::create_dir_all(&root)
            .map_err(|e| MemoryError::Io(format!("创建记忆目录失败：{e}")))?;
        Ok(Self::new(root))
    }

    fn entry_file(&self, path: &MemoryPath) -> PathBuf {
        let mut file = self.root.clone();
        for seg in path.segments() {
            file.push(seg);
        }
        file.set_extension("md");
        file
    }

    fn subtree_dir(&self, path: &MemoryPath) -> PathBuf {
        let mut dir = self.root.clone();
        for seg in path.segments() {
            dir.push(seg);
        }
        dir
    }

    fn list_entries(&self) -> Result<Vec<String>, MemoryError> {
        let mut out = Vec::new();
        Self::walk(&self.root, &self.root, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), MemoryError> {
        for entry in std::fs::read_dir(dir)
            .map_err(|e| MemoryError::Io(format!("读取记忆目录失败 {dir:?}：{e}")))?
        {
            let entry = entry.map_err(|e| MemoryError::Io(format!("读取目录项失败：{e}")))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| MemoryError::Io(format!("读取文件类型失败：{e}")))?;
            if file_type.is_dir() {
                Self::walk(root, &path, out)?;
            } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md")
            {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|_| MemoryError::Io("记忆路径越界".into()))?;
                let mut name = rel.to_string_lossy().replace('\\', "/");
                if let Some(stripped) = name.strip_suffix(".md") {
                    name = stripped.to_string();
                }
                out.push(name);
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MemoryService for FileMemoryService {
    async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError> {
        let file = self.entry_file(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemoryError::Io(format!("创建记忆目录失败：{e}")))?;
        }
        let tmp = file.with_extension("md.tmp");
        std::fs::write(&tmp, content)
            .map_err(|e| MemoryError::Io(format!("写记忆条目失败：{e}")))?;
        std::fs::rename(&tmp, &file)
            .map_err(|e| MemoryError::Io(format!("记忆条目改名失败：{e}")))?;
        Ok(())
    }

    async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
        match path {
            None => Ok(MemoryView::Listing(self.list_entries()?)),
            Some(p) => {
                let file = self.entry_file(p);
                let content = std::fs::read_to_string(&file)
                    .map_err(|_| MemoryError::NotFound(p.as_str()))?;
                Ok(MemoryView::Entry {
                    path: p.clone(),
                    content,
                })
            }
        }
    }

    async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError> {
        let name = path.as_str();
        let file = self.entry_file(path);
        let dir = self.subtree_dir(path);
        let mut removed_any = false;

        if file.is_file() {
            std::fs::remove_file(&file)
                .map_err(|e| MemoryError::Io(format!("删除记忆条目失败：{e}")))?;
            removed_any = true;
        }
        if dir.is_dir() {
            // 只删该子树，绝不碰根目录本身。
            if dir != self.root {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| MemoryError::Io(format!("删除记忆子树失败：{e}")))?;
                removed_any = true;
            }
        }
        if !removed_any {
            return Err(MemoryError::NotFound(name));
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
        assert!(MemoryPath::parse("数学/./函数").is_err());
        assert!(MemoryPath::parse("数学/函数/二次函数").is_ok());
    }

    fn temp_memory() -> (FileMemoryService, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mistake-agent-memory-test-{}",
            uuid::Uuid::new_v4()
        ));
        let root = dir.join("memory");
        (FileMemoryService::new(root.clone()), root)
    }

    #[tokio::test]
    async fn file_memory_crud_and_subtree_remove() {
        let (mem, root) = temp_memory();
        let p1 = MemoryPath::parse("数学/函数/二次函数").unwrap();
        let p2 = MemoryPath::parse("数学/几何").unwrap();
        mem.save(&p1, "顶点公式").await.unwrap();
        mem.save(&p2, "勾股定理").await.unwrap();

        // 详情读取
        match mem.show(Some(&p1)).await.unwrap() {
            MemoryView::Entry { path, content } => {
                assert_eq!(path, p1);
                assert_eq!(content, "顶点公式");
            }
            _ => panic!("应为条目详情"),
        }

        // 清单按路径排序
        match mem.show(None).await.unwrap() {
            MemoryView::Listing(listing) => {
                assert_eq!(
                    listing,
                    vec!["数学/几何".to_string(), "数学/函数/二次函数".to_string()]
                );
            }
            _ => panic!("应为清单"),
        }

        // 删除子树
        mem.remove(&MemoryPath::parse("数学").unwrap())
            .await
            .unwrap();
        match mem.show(None).await.unwrap() {
            MemoryView::Listing(listing) => assert!(listing.is_empty()),
            _ => panic!("应为空清单"),
        }
        assert!(mem.show(Some(&p1)).await.is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn file_memory_persists_across_instances() {
        let (mem, root) = temp_memory();
        let p = MemoryPath::parse("英语/时态/一般现在时").unwrap();
        mem.save(&p, "第三人称单数加 s").await.unwrap();
        drop(mem);

        // 新实例从磁盘恢复
        let mem2 = FileMemoryService::new(root.clone());
        match mem2.show(Some(&p)).await.unwrap() {
            MemoryView::Entry { content, .. } => assert_eq!(content, "第三人称单数加 s"),
            _ => panic!("应为条目详情"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn file_memory_remove_missing_returns_not_found() {
        let (mem, root) = temp_memory();
        let err = mem
            .remove(&MemoryPath::parse("不存在/条目").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
        let _ = std::fs::remove_dir_all(root);
    }
}
