//! practice 模板库：确定性模板分发（几何/代数/英语分组）。

mod algebra;
mod english;
mod geometry;

pub(crate) use geometry::{pythagorean, similar_triangles, triangle_congruence};
pub(crate) use algebra::{absolute_value, factorization, inverse_proportion, linear_equation, linear_function, quadratic_equation, quadratic_function, rational_ops};
pub(crate) use english::{future_simple, past_simple, present_continuous, present_simple};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    #[default]
    Basic,
    Variant,
    Advanced,
    /// 高考真题层：只从随包题库池抽取（真实来源标注，不模型生成）。
    Exam,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GenerateParams {
    /// 知识点（支持：三角形全等判定 / 绝对值 / 一般现在时三单）。
    pub knowledge_point: String,
    /// 难度：基础 / 同类变式 / 综合拔高，默认 basic。
    /// 传 exam 时走高考真题池（池内抽取，非模板/LLM 生成）。
    #[serde(default)]
    pub difficulty: Option<Difficulty>,
}

/// 结构化练习条目：正文、答案、图纸规格同源生成。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PracticeItem {
    pub knowledge_point: String,
    pub template_id: String,
    pub difficulty: Difficulty,
    pub question_text: String,
    pub answer_spec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagram_spec: Option<Value>,
    /// 题目来源：模板/LLM 生成为 None；真题池为试卷标注（如"2021 新高考Ⅰ卷（数学）"）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// 支持的知识点清单（供未匹配时回告模型/用户）。
pub const SUPPORTED_POINTS: &[&str] = &[
    "三角形全等判定",
    "相似三角形",
    "勾股定理",
    "一元一次方程",
    "一元二次方程",
    "一次函数",
    "二次函数",
    "反比例函数",
    "绝对值",
    "有理数运算",
    "因式分解",
    "一般现在时三单",
    "一般过去时",
    "现在进行时",
    "一般将来时",
];

/// 按知识点 + 难度确定性生成练习条目（供 practice::generate 与 exam::compose 共用）。
pub fn build_item(knowledge_point: &str, difficulty: Difficulty) -> Option<PracticeItem> {
    // 真题层：只走池内抽取（真实来源），不走模板与 LLM 生成。
    // 同步路径无 storage：用内置种子兜底（主入口 practice::generate 的 Exam 分支
    // 经 read_pool_json 读运行时数据文件，见 exam_pool.rs）。
    if difficulty == Difficulty::Exam {
        return crate::plugin::practice::exam_pool::draw_from_pool(
            crate::plugin::practice::exam_pool::DEFAULT_POOL_JSON,
            knowledge_point,
            &[],
        );
    }
    let kp = knowledge_point.trim();
    if contains_any(kp, &["三角形全等", "全等"]) {
        Some(triangle_congruence(difficulty))
    } else if contains_any(kp, &["一元二次方程", "二次方程"]) {
        Some(quadratic_equation(difficulty))
    } else if contains_any(kp, &["一元一次方程", "一次方程"]) {
        Some(linear_equation(difficulty))
    } else if contains_any(kp, &["二次函数"]) {
        Some(quadratic_function(difficulty))
    } else if contains_any(kp, &["一次函数"]) {
        Some(linear_function(difficulty))
    } else if contains_any(kp, &["反比例函数", "反比例"]) {
        Some(inverse_proportion(difficulty))
    } else if contains_any(kp, &["勾股定理", "勾股"]) {
        Some(pythagorean(difficulty))
    } else if contains_any(kp, &["相似三角形", "相似"]) {
        Some(similar_triangles(difficulty))
    } else if contains_any(kp, &["因式分解"]) {
        Some(factorization(difficulty))
    } else if contains_any(kp, &["有理数"]) {
        Some(rational_ops(difficulty))
    } else if contains_any(kp, &["绝对值", "绝对"]) {
        Some(absolute_value(difficulty))
    } else if contains_any(kp, &["一般过去时", "过去时"]) {
        Some(past_simple(difficulty))
    } else if contains_any(kp, &["现在进行时", "进行时"]) {
        Some(present_continuous(difficulty))
    } else if contains_any(kp, &["一般将来时", "将来时"]) {
        Some(future_simple(difficulty))
    } else if contains_any(kp, &["一般现在时", "三单", "现在时"]) {
        Some(present_simple(difficulty))
    } else {
        None
    }
}

pub(crate) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}
