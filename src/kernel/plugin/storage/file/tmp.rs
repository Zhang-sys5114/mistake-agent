use super::*;
use crate::kernel::plugin::services::TmpIo;


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
pub(crate) fn rotate_if_large(path: &Path) {
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
