//! practice 插件：分层变式练习（场景二入口，本期提供确定性模板 + diagram_spec）。
//!
//! 设计原则（docs/variants.md）：题目正文、答案与图形三者同源——模板决定骨架，
//! 难度决定参数/条件形态；几何图只出图纸规格（diagram_spec），由前端确定性 SVG 渲染。

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
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GenerateParams {
    /// 知识点（支持：三角形全等判定 / 绝对值 / 一般现在时三单）。
    pub knowledge_point: String,
    /// 难度：基础 / 同类变式 / 综合拔高，默认 basic。
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
}

/// 支持的知识点清单（供未匹配时回告模型/用户）。
pub const SUPPORTED_POINTS: &[&str] = &["三角形全等判定", "绝对值", "一般现在时三单"];

/// 按知识点 + 难度确定性生成练习条目（供 practice::generate 与 exam::compose 共用）。
pub fn build_item(knowledge_point: &str, difficulty: Difficulty) -> Option<PracticeItem> {
    let kp = knowledge_point.trim();
    if contains_any(kp, &["三角形全等", "全等", "三角形"]) {
        Some(triangle_congruence(difficulty))
    } else if contains_any(kp, &["绝对值", "绝对"]) {
        Some(absolute_value(difficulty))
    } else if contains_any(kp, &["一般现在时", "三单", "现在时"]) {
        Some(present_simple(difficulty))
    } else {
        None
    }
}

pub(crate) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

// ---------- 模板一：三角形全等判定（几何，含 diagram_spec） ----------

pub(crate) fn triangle_congruence(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer, diagram) = match difficulty {
        Difficulty::Basic => (
            "triangle_sss",
            "已知 △ABC 与 △DEF 中，AB=DE=3，BC=EF=4，AC=DF=5。判断两个三角形是否全等，并写出判定依据。",
            "全等。三边分别相等（SSS）：AB=DE，BC=EF，AC=DF。",
            Some(sss_diagram()),
        ),
        Difficulty::Variant => (
            "triangle_sas_right",
            "如图，在 △ABC 与 △DEF 中，∠A=∠D=90°，AB=DE=3，AC=DF=4。判断两个三角形是否全等，并写出判定依据。",
            "全等。两边及其夹角分别相等（SAS）：AB=DE，∠A=∠D，AC=DF。",
            Some(sas_diagram()),
        ),
        Difficulty::Advanced => (
            "triangle_isosceles_median",
            "如图，在 △ABC 中，AB=AC，D 是 BC 的中点，连接 AD。证明 △ABD ≌ △ACD。",
            "AB=AC（已知），BD=CD（D 为中点），AD=AD（公共边），三边分别相等（SSS），故 △ABD ≌ △ACD。",
            Some(isosceles_median_diagram()),
        ),
    };
    PracticeItem {
        knowledge_point: "三角形全等判定".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: diagram,
    }
}

pub(crate) fn sss_diagram() -> Value {
    json!({
        "points": {
            "A": [0, 0], "B": [3, 0], "C": [3, 4],
            "D": [7, 0], "E": [10, 0], "F": [10, 4],
        },
        "objects": [
            {"type": "segment", "ends": ["A", "B"]},
            {"type": "segment", "ends": ["B", "C"]},
            {"type": "segment", "ends": ["C", "A"]},
            {"type": "segment", "ends": ["D", "E"]},
            {"type": "segment", "ends": ["E", "F"]},
            {"type": "segment", "ends": ["F", "D"]},
            {"type": "equal_ticks", "ends": ["A", "B"], "count": 1},
            {"type": "equal_ticks", "ends": ["D", "E"], "count": 1},
            {"type": "equal_ticks", "ends": ["B", "C"], "count": 2},
            {"type": "equal_ticks", "ends": ["E", "F"], "count": 2},
            {"type": "equal_ticks", "ends": ["C", "A"], "count": 3},
            {"type": "equal_ticks", "ends": ["F", "D"], "count": 3},
            {"type": "label", "point": "A", "text": "A", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "B", "text": "B", "dx": 0.1, "dy": 0.4},
            {"type": "label", "point": "C", "text": "C", "dx": 0.3, "dy": -0.4},
            {"type": "label", "point": "D", "text": "D", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "E", "text": "E", "dx": 0.1, "dy": 0.4},
            {"type": "label", "point": "F", "text": "F", "dx": 0.3, "dy": -0.4},
        ],
        "labels": ["A", "B", "C", "D", "E", "F"],
    })
}

pub(crate) fn sas_diagram() -> Value {
    json!({
        "points": {
            "A": [0, 0], "B": [3, 0], "C": [0, 4],
            "D": [7, 0], "E": [10, 0], "F": [7, 4],
        },
        "objects": [
            {"type": "segment", "ends": ["A", "B"]},
            {"type": "segment", "ends": ["B", "C"]},
            {"type": "segment", "ends": ["C", "A"]},
            {"type": "segment", "ends": ["D", "E"]},
            {"type": "segment", "ends": ["E", "F"]},
            {"type": "segment", "ends": ["F", "D"]},
            {"type": "right_mark", "vertex": "A", "arm1": "B", "arm2": "C"},
            {"type": "right_mark", "vertex": "D", "arm1": "E", "arm2": "F"},
            {"type": "equal_ticks", "ends": ["A", "B"], "count": 1},
            {"type": "equal_ticks", "ends": ["D", "E"], "count": 1},
            {"type": "equal_ticks", "ends": ["A", "C"], "count": 2},
            {"type": "equal_ticks", "ends": ["D", "F"], "count": 2},
            {"type": "label", "point": "A", "text": "A", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "B", "text": "B", "dx": 0.1, "dy": 0.4},
            {"type": "label", "point": "C", "text": "C", "dx": 0.1, "dy": -0.4},
            {"type": "label", "point": "D", "text": "D", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "E", "text": "E", "dx": 0.1, "dy": 0.4},
            {"type": "label", "point": "F", "text": "F", "dx": 0.1, "dy": -0.4},
        ],
        "labels": ["A", "B", "C", "D", "E", "F"],
    })
}

pub(crate) fn isosceles_median_diagram() -> Value {
    json!({
        "points": {
            "A": [0, 0], "B": [6, 0], "C": [3, 4], "D": [3, 0],
        },
        "objects": [
            {"type": "segment", "ends": ["A", "B"]},
            {"type": "segment", "ends": ["B", "C"]},
            {"type": "segment", "ends": ["C", "A"]},
            {"type": "segment", "ends": ["A", "D"], "dashed": true},
            {"type": "right_mark", "vertex": "D", "arm1": "A", "arm2": "C"},
            {"type": "equal_ticks", "ends": ["A", "B"], "count": 1},
            {"type": "equal_ticks", "ends": ["A", "C"], "count": 1},
            {"type": "equal_ticks", "ends": ["B", "D"], "count": 2},
            {"type": "equal_ticks", "ends": ["D", "C"], "count": 2},
            {"type": "label", "point": "A", "text": "A", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "B", "text": "B", "dx": 0.15, "dy": 0.4},
            {"type": "label", "point": "C", "text": "C", "dx": 0.3, "dy": -0.4},
            {"type": "label", "point": "D", "text": "D", "dx": 0.15, "dy": 0.4},
        ],
        "labels": ["A", "B", "C", "D"],
    })
}

// ---------- 模板二：绝对值 ----------

pub(crate) fn absolute_value(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "abs_evaluate",
            "计算：|-3| + |5| = ?",
            "8（|-3|=3，|5|=5，3+5=8）",
        ),
        Difficulty::Variant => ("abs_reverse", "已知 |x| = 5，且 x < 0，求 x。", "x = -5"),
        Difficulty::Advanced => ("abs_equation", "解方程：|x - 2| = 3。", "x = -1 或 x = 5"),
    };
    PracticeItem {
        knowledge_point: "绝对值".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
    }
}

// ---------- 模板三：一般现在时三单 ----------

pub(crate) fn present_simple(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "present_simple_go",
            "用所给词的适当形式填空：He ___ (go) to school every day.",
            "goes（第三人称单数，一般现在时动词加 -es）",
        ),
        Difficulty::Variant => (
            "present_simple_study",
            "用所给词的适当形式填空：She ___ (study) English every evening.",
            "studies（辅音字母+y 结尾变 y 为 i 加 -es）",
        ),
        Difficulty::Advanced => (
            "present_simple_watch",
            "改错并说明理由：My father watch TV after dinner every day.",
            "watch → watches；主语是第三人称单数，一般现在时动词加 -es",
        ),
    };
    PracticeItem {
        knowledge_point: "一般现在时三单".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
    }
}
