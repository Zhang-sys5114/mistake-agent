//! practice 插件：高考真题池（P2 内容与防重复）。
//!
//! 题库文件随包发布（include_str! 编译期嵌入，离线可用），按学科/知识点/来源标注；
//! 真题走"池内抽取"而非模板/LLM 生成，保证题目真实可溯（来源随条目返回）。

use serde::Deserialize;
use serde_json::Value;

use super::templates::{Difficulty, PracticeItem};

/// 池内真题条目（与 data/gaokao_pool.json 的结构一一对应）。
#[derive(Debug, Clone, Deserialize)]
pub struct ExamItem {
    pub item_id: String,
    /// 试卷来源标注，如"2021 新高考Ⅰ卷（数学）"。
    pub source: String,
    pub subject: String,
    pub knowledge_point: String,
    pub question_text: String,
    pub answer_spec: String,
    #[serde(default)]
    pub diagram_spec: Option<Value>,
}

/// 随包发布的真题池（扩充时直接追加 data/gaokao_pool.json 并重新编译）。
const EXAM_POOL_JSON: &str = include_str!("data/gaokao_pool.json");

/// 从真题池按知识点抽取一道真题。
///
/// 匹配规则：请求知识点与条目知识点双向包含（如"集合"可命中"集合运算"）；
/// 命中多条时随机抽取（uuid 作无依赖伪随机），避免重复出同一道。
pub fn draw_from_pool(knowledge_point: &str) -> Option<PracticeItem> {
    let pool: Vec<ExamItem> = serde_json::from_str(EXAM_POOL_JSON).ok()?;
    let kp = knowledge_point.trim();
    if kp.is_empty() {
        return None;
    }
    let matched: Vec<&ExamItem> = pool
        .iter()
        .filter(|e| e.knowledge_point.contains(kp) || kp.contains(e.knowledge_point.as_str()))
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

    #[test]
    fn draw_matches_by_knowledge_point() {
        let item = draw_from_pool("集合").expect("题库应包含集合题");
        assert_eq!(item.difficulty, Difficulty::Exam);
        assert!(item.template_id.starts_with("exam:"));
        assert!(item.source.as_deref().unwrap().contains("卷"));
        assert!(!item.question_text.is_empty());
        assert!(!item.answer_spec.is_empty());
    }

    #[test]
    fn draw_misses_unknown_point() {
        assert!(draw_from_pool("量子力学").is_none());
    }

    #[test]
    fn draw_ignores_empty_point() {
        assert!(draw_from_pool("  ").is_none());
    }
}
