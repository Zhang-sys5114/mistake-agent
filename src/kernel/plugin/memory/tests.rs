use super::*;
use crate::kernel::plugin::storage::FileStorage;
use std::collections::BTreeMap;
use std::sync::Mutex;

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

fn temp_memory() -> (FileMemoryService, Arc<FileStorage>) {
    // 文件版走真实文件后端（临时数据根），验证编码/落盘/跨实例恢复。
    let dir = std::env::temp_dir().join(format!(
        "mistake-agent-memory-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(dir.join("memory")).unwrap();
    let store = Arc::new(FileStorage::open(&dir).unwrap());
    let io: Arc<dyn DomainIo> = store.clone();
    (FileMemoryService::new(io), store)
}

#[tokio::test]
async fn file_memory_crud_and_subtree_remove() {
    let (mem, _store) = temp_memory();
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
}

#[tokio::test]
async fn file_memory_persists_across_instances() {
    let (mem, store) = temp_memory();
    let p = MemoryPath::parse("英语/时态/一般现在时").unwrap();
    mem.save(&p, "第三人称单数加 s").await.unwrap();
    drop(mem);

    // 新实例从磁盘恢复（同一文件后端）。
    let io: Arc<dyn DomainIo> = store.clone();
    let mem2 = FileMemoryService::new(io);
    match mem2.show(Some(&p)).await.unwrap() {
        MemoryView::Entry { content, .. } => assert_eq!(content, "第三人称单数加 s"),
        _ => panic!("应为条目详情"),
    }
}

#[tokio::test]
async fn file_memory_remove_missing_returns_not_found() {
    let (mem, _store) = temp_memory();
    let err = mem
        .remove(&MemoryPath::parse("不存在/条目").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, MemoryError::NotFound(_)));
}

#[tokio::test]
async fn migrate_legacy_layout_moves_chinese_paths_to_encoded() {
    let (_mem, store) = temp_memory();
    // 模拟旧布局：中文路径直接落盘。
    let io: Arc<dyn DomainIo> = store.clone();
    io.write(Domain::Memory, &RelPath::parse("seed.md").unwrap(), b"").await.unwrap();
    // 直接经文件系统写旧布局文件。
    let root = std::env::temp_dir()
        .join(format!("mistake-agent-memory-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("memory").join("测试")).unwrap();
    std::fs::write(root.join("memory/测试/记忆条目.md"), "顶点公式").unwrap();
    let store2: Arc<FileStorage> = Arc::new(FileStorage::open(&root).unwrap());
    let mem2 = FileMemoryService::new(store2.clone());
    let io2: Arc<dyn DomainIo> = store2.clone();

    mem2.migrate_legacy_layout().await.unwrap();
    // 旧文件已删、新编码文件可读、内容一致。
    let rels = io2.list(Domain::Memory).await.unwrap();
    assert!(!rels.iter().any(|r| r == "测试/记忆条目.md"), "旧布局文件应被迁移删除");
    let mut found = false;
    for rel in rels {
        if let Some(name) = rel_to_memory_name(&rel)
            && name == "测试/记忆条目"
        {
            found = true;
            let bytes = io2.read(Domain::Memory, &RelPath::parse(&rel).unwrap()).await.unwrap();
            assert_eq!(String::from_utf8(bytes).unwrap(), "顶点公式");
        }
    }
    assert!(found, "新布局应能读到迁移后的条目");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- 入口 handler 测试（原 plugin/memory 迁移，ADR-0035） ----------

#[derive(Default)]
struct FakeMemory {
    entries: Mutex<BTreeMap<String, String>>,
}

#[async_trait]
impl MemoryService for FakeMemory {
    async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError> {
        self.entries
            .lock()
            .expect("poisoned")
            .insert(path.as_str(), content.to_string());
        Ok(())
    }
    async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
        let entries = self.entries.lock().expect("poisoned");
        match path {
            None => Ok(MemoryView::Listing(entries.keys().cloned().collect())),
            Some(p) => entries
                .get(&p.as_str())
                .cloned()
                .map(|content| MemoryView::Entry {
                    path: p.clone(),
                    content,
                })
                .ok_or_else(|| MemoryError::NotFound(p.as_str())),
        }
    }
    async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError> {
        let prefix = format!("{}/", path.as_str());
        let mut entries = self.entries.lock().expect("poisoned");
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

fn handle() -> MemoryHandle {
    MemoryHandle::new(Arc::new(FakeMemory::default()))
}

#[tokio::test]
async fn save_show_roundtrip() {
    let mem = handle();
    let out = save_handler(
        mem.clone(),
        json!({"filename": "数学/函数/二次函数 顶点公式", "content": "顶点公式 x=-b/2a"}),
    )
    .await
    .unwrap();
    assert_eq!(out["ok"], true);

    let listing = show_handler(mem.clone(), json!({})).await.unwrap();
    assert_eq!(listing["count"], 1);
    assert_eq!(listing["entries"][0], "数学/函数/二次函数 顶点公式");

    let detail = show_handler(
        mem.clone(),
        json!({"filename": "数学/函数/二次函数 顶点公式"}),
    )
    .await
    .unwrap();
    assert!(detail["content"].as_str().unwrap().contains("顶点公式"));
}

#[tokio::test]
async fn remove_requires_and_deletes_subtree() {
    let mem = handle();
    save_handler(
        mem.clone(),
        json!({"filename": "数学/几何/勾股", "content": "a²+b²=c²"}),
    )
    .await
    .unwrap();
    save_handler(
        mem.clone(),
        json!({"filename": "数学/几何/相似", "content": "对应边成比例"}),
    )
    .await
    .unwrap();

    let err = remove_handler(mem.clone(), json!({})).await.unwrap_err();
    assert_eq!(
        err.code,
        crate::kernel::contract::ToolErrorCode::InvalidParams
    );

    let out = remove_handler(mem.clone(), json!({"filename": "数学/几何"}))
        .await
        .unwrap();
    assert_eq!(out["ok"], true);
    let listing = show_handler(mem, json!({})).await.unwrap();
    assert_eq!(listing["count"], 0);
}

#[tokio::test]
async fn path_escape_rejected() {
    let mem = handle();
    let err = save_handler(mem, json!({"filename": "数学/../秘密", "content": "x"}))
        .await
        .unwrap_err();
    assert_eq!(
        err.code,
        crate::kernel::contract::ToolErrorCode::InvalidParams
    );
}
