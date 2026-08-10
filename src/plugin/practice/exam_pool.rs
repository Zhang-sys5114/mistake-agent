//! practice 插件：高考真题池（P2 内容与防重复）。
//!
//! ADR-0042 数据运行时化：真题池从编译期 include_str! 改为运行时数据文件
//! （`data/gaokao_pool.json`，经 StorageHandle 读写）；缺失/损坏时回退内置种子
//! `DEFAULT_POOL_JSON`（离线兜底，过渡平滑）。运行时可被模型/家长经 storage 更新，
//! 更新不依赖发版。真题走"池内抽取"而非模板/LLM 生成，保证题目真实可溯（来源随条目返回）。

use serde::Deserialize;
use serde_json::Value;

use super::templates::{Difficulty, PracticeItem};

/// 池内真题条目（与 data/gaokao_pool.json 的结构一一对应）。
#[derive(Debug, Clone, Deserialize)]
pub struct ExamItem {
    pub item_id: String,
    /// 试卷来源标注，如"2021 新高考Ⅰ卷（数学）"。
    pub source: String,
    // 数据文件中的 subject（学科）标注保留用于人工筛选与未来扩展，
    // 结构体不读取（serde 忽略未知字段），避免死代码。
    pub knowledge_point: String,
    pub question_text: String,
    pub answer_spec: String,
    #[serde(default)]
    pub diagram_spec: Option<Value>,
}

/// 内置种子真题池（离线兜底；运行时数据文件优先，见 `read_pool_json`）。
pub const DEFAULT_POOL_JSON: &str = include_str!("data/gaokao_pool.json");

/// 真题池运行时文件名（数据根目录 data/ 下）。
pub const POOL_FILE_NAME: &str = "gaokao_pool.json";

/// 读取真题池 JSON：运行时数据文件优先，缺失/损坏回退内置种子（不报错——
/// 出题不能因数据问题崩，与旧版"解析失败返回 None"行为一致）。
/// "损坏"含两层：文件不可读（缺失/IO 错/非 UTF-8）与解析失败（非 JSON / 非数组）。
pub async fn read_pool_json(storage: &crate::kernel::plugin::services::StorageHandle) -> String {
    match storage.read_data_file(POOL_FILE_NAME).await {
        Ok(json) if !json.trim().is_empty() && serde_json::from_str::<Vec<ExamItem>>(&json).is_ok() => {
            json
        }
        _ => DEFAULT_POOL_JSON.to_string(),
    }
}

/// 从真题池按知识点抽取一道真题。
///
/// 匹配规则：请求知识点与条目知识点双向包含（如"集合"可命中"集合运算"）；
/// exclude 为近期已掌握题目标识（template_id 格式 exam:item_id），抽题时避开；
/// 命中多条时随机抽取（uuid 作无依赖伪随机），避免重复出同一道。
pub fn draw_from_pool(
    pool_json: &str,
    knowledge_point: &str,
    exclude: &[String],
) -> Option<PracticeItem> {
    let pool: Vec<ExamItem> = serde_json::from_str(pool_json).ok()?;
    let kp = knowledge_point.trim();
    if kp.is_empty() {
        return None;
    }
    let matched: Vec<&ExamItem> = pool
        .iter()
        .filter(|e| e.knowledge_point.contains(kp) || kp.contains(e.knowledge_point.as_str()))
        .filter(|e| !exclude.iter().any(|x| x == &format!("exam:{}", e.item_id)))
        .collect();
    if matched.is_empty() {
        return None;
    }
    let idx = uuid::Uuid::new_v4().as_u128() as usize % matched.len();
    let item = matched[idx];
    Some(PracticeItem {
        knowledge_point: item.knowledge_point.clone(),
        template_id: format!("exam:{}", item.item_id),
        difficulty: Difficulty::Exam,
        question_text: item.question_text.clone(),
        answer_spec: item.answer_spec.clone(),
        diagram_spec: item.diagram_spec.clone(),
        source: Some(item.source.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::plugin::services::StorageHandle;
    use crate::kernel::plugin::storage::FileStorage;
    use std::sync::Arc;

    /// 真实文件后端 + 临时数据根：走真实磁盘链路（data/gaokao_pool.json 落盘/读取）。
    fn real_storage(dir_name: &str) -> (StorageHandle, Arc<FileStorage>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mistake-agent-{dir_name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let store: Arc<FileStorage> = Arc::new(FileStorage::open(&dir).unwrap());
        let handle = StorageHandle::new(store.clone()).with_io(store.clone(), store.clone());
        (handle, store, dir)
    }

    #[tokio::test]
    async fn read_pool_prefers_runtime_file_over_seed() {
        let (storage, _store, dir) = real_storage("pool-prefers");
        // 写入一个自定义运行时池（只有"复数"知识点），验证文件优先于内置种子。
        storage
            .write_data_file(
                POOL_FILE_NAME,
                r#"[{"item_id":"rt1","source":"运行时补充","knowledge_point":"复数","question_text":"i^2=?","answer_spec":"-1"}]"#,
            )
            .await
            .unwrap();
        let json = read_pool_json(&storage).await;
        assert!(json.contains("运行时补充"));
        let item = draw_from_pool(&json, "复数", &[]).expect("运行时池应命中复数");
        assert_eq!(item.template_id, "exam:rt1");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn read_pool_falls_back_to_seed_when_missing() {
        let (storage, _store, dir) = real_storage("pool-missing");
        let json = read_pool_json(&storage).await;
        // 缺失运行时文件 → 内置种子（含"集合"题）。
        assert!(json.contains("集合"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn read_pool_falls_back_to_seed_when_corrupt() {
        let (storage, _store, dir) = real_storage("pool-corrupt");
        storage
            .write_data_file(POOL_FILE_NAME, "not json")
            .await
            .unwrap();
        let json = read_pool_json(&storage).await;
        assert!(json.contains("集合"), "损坏文件应回退内置种子");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn draw_matches_by_knowledge_point() {
        let item = draw_from_pool(DEFAULT_POOL_JSON, "集合", &[]).expect("题库应包含集合题");
        assert_eq!(item.difficulty, Difficulty::Exam);
        assert!(item.template_id.starts_with("exam:"));
        assert!(item.source.as_deref().unwrap().contains("卷"));
        assert!(!item.question_text.is_empty());
        assert!(!item.answer_spec.is_empty());
    }

    #[test]
    fn draw_misses_unknown_point() {
        assert!(draw_from_pool(DEFAULT_POOL_JSON, "量子力学", &[]).is_none());
    }

    #[test]
    fn draw_ignores_empty_point() {
        assert!(draw_from_pool(DEFAULT_POOL_JSON, "  ", &[]).is_none());
    }

    #[test]
    fn draw_skips_mastered_items() {
        // 先用空排除拿到实际 item_id，再排除它。
        let first = draw_from_pool(DEFAULT_POOL_JSON, "集合", &[]).expect("题库应包含集合题");
        let exclude = vec![first.template_id.clone()];
        let second = draw_from_pool(DEFAULT_POOL_JSON, "集合", &exclude);
        if let Some(item) = second {
            assert_ne!(item.template_id, first.template_id);
        }
        // 排除全部集合题（把池内 6 道 id 全列上）应返回 None。
        let all: Vec<String> = [
            "gk2020gj1_p1", "gk2021new1_p1", "gk2021new1_p2", "gk2022gj2_p1", "gk2022new1_p1",
            "gk2023gj1_p1",
        ]
        .iter()
        .map(|id| format!("exam:{id}"))
        .collect();
        assert!(draw_from_pool(DEFAULT_POOL_JSON, "集合", &all).is_none());
    }

    #[test]
    fn draw_parses_invalid_pool_as_none() {
        assert!(draw_from_pool("not json", "集合", &[]).is_none());
    }
}
