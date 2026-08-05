//! memory 服务（ADR-0008）：目录式记忆路由。
//! 内核插件信息：ServiceId::Memory；能力 = save/show/remove（路径校验、子树删除）；
//! 生产实现 = 文件持久化（数据根 memory/），InMemoryMemory 保留供测试。
//! - `InMemoryMemory`：测试/内存回退实现；
//! - `FileMemoryService`：数据根目录 memory/ 下的文件持久化（M2 生产实现）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::KernelContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::{
    MemoryError, MemoryHandle, MemoryPath, MemoryService, MemoryView, ServiceId,
};
use crate::kernel::registry::{KernelDescriptor, KernelPlugin};
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
        // memory/ 目录已由 bootstrap::init_data_root（Kernel::new 引导）创建。
        Ok(Self::new(Settings::data_root().join("memory")))
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

// ---------- 内核插件入口（ADR-0035）：memory::save/show/remove ----------
//
// 路径校验、文件持久化、事件与审计由本模块 MemoryService / MemoryHandle 负责；
// 入口层只做参数解析与错误映射（ADR-0008）。工具经 KernelPlugin 契约注册，
// 与用户插件同表校验（namespace/wire 唯一、CallerPolicy）。

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SaveParams {
    /// 记忆路径（层级：学科/知识点/条目），如"数学/函数/二次函数 顶点公式"；缺省 = 会话/自动保存。
    pub filename: Option<String>,
    /// 要记住的内容（一句话结论或要点）；缺省 = 由模型总结当前会话上下文后填入。
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ShowParams {
    /// 可选：省略 = 列出全部记忆条目名；带 = 查看具体条目内容。
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RemoveParams {
    /// 要删除的记忆条目或整棵子树路径。
    pub filename: String,
}

pub struct MemoryPlugin;

impl KernelPlugin for MemoryPlugin {
    fn info() -> Info {
        Info {
            namespace: "memory".into(),
            provides: vec![ServiceId::Memory],
            tools: vec![
                ToolDef {
                    name: "save".into(),
                    user_visible: true,
                    title: Some("记住一条笔记".into()),
                    group: Some("记忆".into()),
                    description: "把重要结论/学习要点保存到跨会话记忆。用法：memory::save <想保存的内容>（可选参数；内容缺省时请总结当前会话要点填入 content）".into(),
                    params: schemars::schema_for!(SaveParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: Some("mdi:content-save".into()),
                },
                ToolDef {
                    name: "show".into(),
                    user_visible: true,
                    title: Some("翻看记忆".into()),
                    group: Some("记忆".into()),
                    description: "查看记忆：用法 memory::show <记忆片段>（可选参数；省略 = 列出全部条目名）".into(),
                    params: schemars::schema_for!(ShowParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: Some("mdi:book-open-variant".into()),
                },
                ToolDef {
                    name: "remove".into(),
                    user_visible: true,
                    title: Some("删除记忆".into()),
                    group: Some("记忆".into()),
                    description: "删除记忆条目或整棵子树（仅用户可调，需确认）。用法：memory::remove <记忆片段>".into(),
                    params: schemars::schema_for!(RemoveParams),
                    policy: CallerPolicy::UserOnly,
                    timeout: None,
                    icon: Some("mdi:delete".into()),
                },
            ],
            ..Default::default()
        }
    }

    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError> {
        let memory = ctx
            .handles
            .memory()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Memory 句柄".into()))?;

        let mem_save = memory.clone();
        ctx.registrar.tool(
            "save",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let memory = mem_save.clone();
                Box::pin(async move { save_handler(memory, params).await })
            }),
        )?;

        let mem_show = memory.clone();
        ctx.registrar.tool(
            "show",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let memory = mem_show.clone();
                Box::pin(async move { show_handler(memory, params).await })
            }),
        )?;

        let mem_remove = memory.clone();
        ctx.registrar.tool(
            "remove",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let memory = mem_remove.clone();
                Box::pin(async move { remove_handler(memory, params).await })
            }),
        )
    }
}

pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<MemoryPlugin>()
}

async fn save_handler(memory: MemoryHandle, params: Value) -> Result<Value, ToolError> {
    let p: SaveParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let filename = p.filename.unwrap_or_else(|| "会话/自动保存".into());
    let content = p.content.unwrap_or_default();
    if content.trim().is_empty() {
        return Err(ToolError::handler(
            "未提供要保存的内容。用法：memory::save <想保存的内容>；也可以由模型总结当前会话要点后填入 content。",
        ));
    }
    let path = MemoryPath::parse(&filename).map_err(map_memory_error)?;
    memory
        .save(&path, &content)
        .await
        .map_err(map_memory_error)?;
    Ok(json!({ "ok": true, "path": path.as_str() }))
}

async fn show_handler(memory: MemoryHandle, params: Value) -> Result<Value, ToolError> {
    let p: ShowParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let path = p
        .filename
        .as_deref()
        .map(MemoryPath::parse)
        .transpose()
        .map_err(map_memory_error)?;
    match memory.show(path.as_ref()).await.map_err(map_memory_error)? {
        MemoryView::Listing(entries) => Ok(json!({
            "count": entries.len(),
            "entries": entries,
        })),
        MemoryView::Entry { path, content } => Ok(json!({
            "path": path.as_str(),
            "content": content,
        })),
    }
}

async fn remove_handler(memory: MemoryHandle, params: Value) -> Result<Value, ToolError> {
    let p: RemoveParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    if p.filename.trim().is_empty() {
        return Err(ToolError::invalid_params(
            "remove 必须提供 filename 以确认删除目标",
        ));
    }
    let path = MemoryPath::parse(&p.filename).map_err(map_memory_error)?;
    memory.remove(&path).await.map_err(map_memory_error)?;
    Ok(json!({ "ok": true, "removed": path.as_str() }))
}

fn map_memory_error(e: MemoryError) -> ToolError {
    match e {
        MemoryError::InvalidPath(m) => ToolError::invalid_params(m),
        MemoryError::NotFound(m) => ToolError::handler(format!("记忆条目不存在：{m}")),
        MemoryError::Io(m) => ToolError::handler(format!("记忆读写失败：{m}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
