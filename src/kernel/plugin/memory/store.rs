//! 文件版记忆（DomainIo memory 域，ADR-0042）。

use super::*;

pub struct FileMemoryService {
    io: Arc<dyn DomainIo>,
}

impl FileMemoryService {
    pub fn new(io: Arc<dyn DomainIo>) -> Self {
        Self { io }
    }

    pub fn open_default(io: Arc<dyn DomainIo>) -> Result<Self, MemoryError> {
        // memory/ 目录已由 bootstrap::init_data_root（Kernel::new 引导）创建。
        Ok(Self::new(io))
    }

    /// 旧存储布局迁移（ADR-0042 数据运行时化连坐）：v1 的 memory 条目以
    /// 中文路径直接落盘（`memory/测试/记忆条目.md`），新布局为 base64url 段编码
    /// （`memory/<enc>/<enc>.md`）。启动时把旧条目读入→按新布局写出→删旧文件。
    /// 幂等：新布局条目（段可解码）原样保留；旧布局条目（段含非 base64 字符）迁移。
    pub async fn migrate_legacy_layout(&self) -> Result<(), MemoryError> {
        let rels = self
            .io
            .list(Domain::Memory)
            .await
            .map_err(|e| MemoryError::Io(format!("迁移扫描失败：{e}")))?;
        for rel in rels {
            if rel_to_memory_name(&rel).is_some() {
                continue; // 已是新布局（每段可解码）。
            }
            // 旧布局：每段剥掉尾部 .md（那是文件系统扩展名）后原样编码为新段。
            let mut encoded = Vec::new();
            for seg in rel.split('/') {
                if seg.is_empty() {
                    continue;
                }
                encoded.push(encode_segment(seg.strip_suffix(".md").unwrap_or(seg)));
            }
            if encoded.is_empty() {
                continue;
            }
            let new_rel = RelPath::parse(&format!("{}.md", encoded.join("/")))
                .map_err(|e| MemoryError::Io(format!("迁移编码异常：{e}")))?;
            let bytes = self
                .io
                .read_legacy(Domain::Memory, &rel)
                .await
                .map_err(|e| MemoryError::Io(format!("迁移读取旧条目失败 {rel}：{e}")))?;
            self.io
                .write(Domain::Memory, &new_rel, &bytes)
                .await
                .map_err(|e| MemoryError::Io(format!("迁移写入新条目失败 {}：{e}", new_rel.as_str())))?;
            self.io
                .remove_legacy(Domain::Memory, &rel)
                .await
                .map_err(|e| MemoryError::Io(format!("迁移删除旧条目失败 {rel}：{e}")))?;
        }
        Ok(())
    }

    fn entry_rel(path: &MemoryPath) -> String {
        let encoded: Vec<String> = path
            .segments()
            .iter()
            .map(|s| encode_segment(s))
            .collect();
        format!("{}.md", encoded.join("/"))
    }

    fn subtree_rel(path: &MemoryPath) -> String {
        let encoded: Vec<String> = path
            .segments()
            .iter()
            .map(|s| encode_segment(s))
            .collect();
        encoded.join("/")
    }
}

/// 段编码：base64url（URL_SAFE_NO_PAD），字符集 [A-Za-z0-9_-]，首尾均为字母数字，
/// 天然满足 RelPath 白名单（段以字母数字开头结尾，中间仅 [a-zA-Z0-9._-]）。
fn encode_segment(seg: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(seg.as_bytes())
}

fn decode_segment(seg: &str) -> Result<String, MemoryError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(seg)
        .map_err(|e| MemoryError::Io(format!("记忆路径解码失败：{e}")))?;
    String::from_utf8(bytes).map_err(|e| MemoryError::Io(format!("记忆路径非 UTF-8：{e}")))
}

pub(crate) fn rel_to_memory_name(rel: &str) -> Option<String> {
    // rel 形如 "BASE64/BASE64.md"；去掉 .md 后缀后逐段解码。
    let no_ext = rel.strip_suffix(".md")?;
    let mut out = Vec::new();
    for seg in no_ext.split('/') {
        out.push(decode_segment(seg).ok()?);
    }
    Some(out.join("/"))
}

#[async_trait]
impl MemoryService for FileMemoryService {
    async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError> {
        let rel = Self::entry_rel(path);
        let rel = RelPath::parse(&rel)
            .map_err(|e| MemoryError::Io(format!("记忆路径编码异常：{e}")))?;
        self.io
            .write(Domain::Memory, &rel, content.as_bytes())
            .await
            .map_err(|e| MemoryError::Io(format!("写记忆条目失败：{e}")))
    }

    async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
        match path {
            None => {
                let rels = self
                    .io
                    .list(Domain::Memory)
                    .await
                    .map_err(|e| MemoryError::Io(format!("列出记忆失败：{e}")))?;
                let mut names = Vec::new();
                for rel in rels {
                    if let Some(name) = rel_to_memory_name(&rel) {
                        names.push(name);
                    }
                }
                names.sort();
                Ok(MemoryView::Listing(names))
            }
            Some(p) => {
                let rel = Self::entry_rel(p);
                let rel = RelPath::parse(&rel)
                    .map_err(|e| MemoryError::Io(format!("记忆路径编码异常：{e}")))?;
                let bytes = self
                    .io
                    .read(Domain::Memory, &rel)
                    .await
                    .map_err(|_| MemoryError::NotFound(p.as_str()))?;
                let content = String::from_utf8(bytes)
                    .map_err(|e| MemoryError::Io(format!("记忆内容非 UTF-8：{e}")))?;
                Ok(MemoryView::Entry {
                    path: p.clone(),
                    content,
                })
            }
        }
    }

    async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError> {
        let name = path.as_str();
        let mut removed_any = false;

        // 先删单条目文件（不存在不报错）。
        let entry = Self::entry_rel(path);
        if let Ok(rel) = RelPath::parse(&entry)
            && self.io.remove(Domain::Memory, &rel).await.is_ok()
        {
            removed_any = true;
        }
        // 再删子树（目录；指向域根时拒绝）。
        let sub = Self::subtree_rel(path);
        if let Ok(rel) = RelPath::parse(&sub)
            && self.io.remove_tree(Domain::Memory, &rel).await.is_ok()
        {
            removed_any = true;
        }
        if !removed_any {
            return Err(MemoryError::NotFound(name));
        }
        Ok(())
    }
}
