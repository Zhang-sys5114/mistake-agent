use super::*;


// ---------- 域内文件 IO（ADR-0042） ----------

use crate::kernel::plugin::services::{Domain, DomainIo, RelPath};

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
        // tmp 名带 uuid：多线程并发写同一目标时互不踩踏（固定名会互相覆盖/误删）。
        let tmp = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
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
    pub(crate) fn staged_path_allowed(&self, path: &Path) -> Result<PathBuf, StorageError> {
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
