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
    if difficulty == Difficulty::Exam {
        return crate::plugin::practice::exam_pool::draw_from_pool(knowledge_point);
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
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "三角形全等判定".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: diagram,
        source: None,
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
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "绝对值".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
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
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "一般现在时三单".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板四：一元一次方程 ----------

pub(crate) fn linear_equation(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "linear_equation_basic",
            "解方程：2x + 5 = 13。",
            "x = 4（移项：2x = 8，系数化为 1）",
        ),
        Difficulty::Variant => (
            "linear_equation_brackets",
            "解方程：3(x - 2) = 2(x + 1)。",
            "x = 8（去括号：3x - 6 = 2x + 2，移项合并得 x = 8）",
        ),
        Difficulty::Advanced => (
            "linear_equation_fractions",
            "解方程：(2x - 1)/3 = (x + 2)/2 - 1。",
            "x = 2（去分母：2(2x - 1) = 3(x + 2) - 6，化简 4x - 2 = 3x，得 x = 2）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "一元一次方程".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板五：一元二次方程 ----------

pub(crate) fn quadratic_equation(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "quadratic_factor",
            "解方程：$x^2 - 5x + 6 = 0$。",
            "$x = 2$ 或 $x = 3$（因式分解：$(x-2)(x-3)=0$）",
        ),
        Difficulty::Variant => (
            "quadratic_square_root",
            "解方程：$2x^2 - 8 = 0$。",
            "$x = 2$ 或 $x = -2$（开平方法：$x^2 = 4$）",
        ),
        Difficulty::Advanced => (
            "quadratic_complete_square",
            "用配方法解方程：$x^2 - 2x - 3 = 0$。",
            "$x = 3$ 或 $x = -1$（配方：$(x-1)^2 = 4$）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "一元二次方程".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板六：一次函数 ----------

pub(crate) fn linear_function(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "linear_function_determine",
            "已知一次函数 $y = kx + b$ 的图像经过点 (1, 3) 和 (0, 1)，求该函数解析式。",
            "$y = 2x + 1$（代入两点：$k=2$，$b=1$）",
        ),
        Difficulty::Variant => (
            "linear_function_quadrant",
            "一次函数 $y = 3x + b$ 的图像经过点 (2, 5)，求 $b$，并判断图像经过哪几个象限。",
            "$b = -1$，图像经过第一、三、四象限（$k=3>0$，截距 $-1<0$）",
        ),
        Difficulty::Advanced => (
            "linear_function_triangle",
            "一次函数 $y = kx + b$ 的图像与 x 轴交于点 (2, 0)，与 y 轴交于点 (0, -4)，求解析式及图像与坐标轴围成的三角形面积。",
            "$y = 2x - 4$，面积 $= \\frac{1}{2} \\times 2 \\times 4 = 4$",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "一次函数".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板七：二次函数 ----------

pub(crate) fn quadratic_function(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "quadratic_function_vertex_basic",
            "抛物线 $y = x^2$ 的顶点坐标和对称轴分别是什么？",
            "顶点 (0, 0)，对称轴 $x = 0$（y 轴）",
        ),
        Difficulty::Variant => (
            "quadratic_function_vertex_form",
            "抛物线 $y = (x - 1)^2 + 2$ 的顶点坐标和对称轴分别是什么？",
            "顶点 (1, 2)，对称轴 $x = 1$（顶点式 $y = a(x-h)^2 + k$）",
        ),
        Difficulty::Advanced => (
            "quadratic_function_general",
            "求抛物线 $y = x^2 - 4x + 3$ 的顶点坐标及与 x 轴的交点坐标。",
            "顶点 (2, -1)（配方：$y = (x-2)^2 - 1$）；与 x 轴交于 (1, 0)、(3, 0)",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "二次函数".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板八：反比例函数 ----------

pub(crate) fn inverse_proportion(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "inverse_proportion_evaluate",
            "反比例函数 $y = \\frac{6}{x}$ 的图像经过点 (2, a)，求 a 的值。",
            "$a = 3$（代入：$a = \\frac{6}{2}$）",
        ),
        Difficulty::Variant => (
            "inverse_proportion_determine",
            "已知反比例函数的图像经过点 (3, 4)，求该反比例函数解析式。",
            "$y = \\frac{12}{x}$（$k = 3 \\times 4 = 12$）",
        ),
        Difficulty::Advanced => (
            "inverse_proportion_intersection",
            "反比例函数 $y = \\frac{k}{x}$ 与一次函数 $y = x + 1$ 的图像交于点 (2, 3) 和另一点，求 k 的值及另一点的坐标。",
            "$k = 6$，另一点为 $(-3, -2)$（联立：$\\frac{6}{x} = x + 1$，解得 $x = 2$ 或 $x = -3$）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "反比例函数".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板九：勾股定理（几何，含 diagram_spec） ----------

pub(crate) fn pythagorean(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer, diagram) = match difficulty {
        Difficulty::Basic => (
            "pythagorean_345",
            "如图，在直角三角形中，两条直角边分别为 3 和 4，求斜边的长。",
            "斜边长为 5（勾股定理：$\\sqrt{3^2 + 4^2} = 5$）",
            Some(pythagorean_diagram(3.0, 4.0)),
        ),
        Difficulty::Variant => (
            "pythagorean_51213",
            "如图，在直角三角形中，两条直角边分别为 5 和 12，求斜边的长。",
            "斜边长为 13（勾股定理：$\\sqrt{5^2 + 12^2} = 13$）",
            Some(pythagorean_diagram(5.0, 12.0)),
        ),
        Difficulty::Advanced => (
            "pythagorean_isosceles_right",
            "如图，等腰直角三角形的斜边长为 $6\\sqrt{2}$，求直角边的长，并判断以 6、8、10 为三边的三角形是否为直角三角形。",
            "直角边长为 6（$a^2 + a^2 = (6\\sqrt{2})^2$，$a = 6$）；6、8、10 满足 $6^2 + 8^2 = 10^2$，是直角三角形（勾股定理逆定理）",
            Some(pythagorean_diagram(6.0, 6.0)),
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "勾股定理".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: diagram,
        source: None,
    }
}

/// 直角三角形图纸规格：直角顶点在原点，两直角边沿坐标轴。
pub(crate) fn pythagorean_diagram(a: f64, b: f64) -> Value {
    json!({
        "points": {
            "A": [a, 0.0], "B": [0.0, b], "C": [0.0, 0.0],
        },
        "objects": [
            {"type": "segment", "ends": ["C", "A"]},
            {"type": "segment", "ends": ["C", "B"]},
            {"type": "segment", "ends": ["A", "B"]},
            {"type": "right_mark", "vertex": "C", "arm1": "A", "arm2": "B"},
            {"type": "label", "point": "A", "text": "A", "dx": 0.2, "dy": 0.3},
            {"type": "label", "point": "B", "text": "B", "dx": -0.45, "dy": 0.2},
            {"type": "label", "point": "C", "text": "C", "dx": -0.45, "dy": 0.45},
        ],
        "labels": ["A", "B", "C"],
    })
}

// ---------- 模板十：相似三角形（几何，含 diagram_spec） ----------

pub(crate) fn similar_triangles(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer, diagram) = match difficulty {
        Difficulty::Basic => (
            "similar_triangles_ratio",
            "如图，△ABC ∽ △DEF，AB = 4，DE = 2，BC = 6，求 EF 的长。",
            "EF = 3（相似比 $\\frac{AB}{DE} = 2$，$EF = \\frac{BC}{2} = 3$）",
            Some(similar_ratio_diagram()),
        ),
        Difficulty::Variant => (
            "similar_triangles_a_shape",
            "如图，在 △ABC 中，DE ∥ BC，AD = 2，DB = 4，DE = 3，求 BC 的长。",
            "BC = 9（△ADE ∽ △ABC，相似比 $\\frac{AD}{AB} = \\frac{2}{6} = \\frac{1}{3}$，$BC = 3 \\times 3 = 9$）",
            Some(similar_a_shape_diagram()),
        ),
        Difficulty::Advanced => (
            "similar_triangles_area_ratio",
            "两个相似三角形的相似比为 2 : 3，它们的面积比是多少？若较小三角形面积为 8，求较大三角形的面积。",
            "面积比为 $4 : 9$（面积比 = 相似比的平方）；较大三角形面积为 $8 \\times \\frac{9}{4} = 18$",
            None,
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "相似三角形".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: diagram,
        source: None,
    }
}

/// 相似比 2:1 的两个三角形并排图纸。
pub(crate) fn similar_ratio_diagram() -> Value {
    json!({
        "points": {
            "A": [0, 0], "B": [6, 0], "C": [2, 4],
            "D": [8, 0], "E": [11, 0], "F": [9, 2],
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
            {"type": "label", "point": "A", "text": "A", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "B", "text": "B", "dx": 0.15, "dy": 0.4},
            {"type": "label", "point": "C", "text": "C", "dx": 0.3, "dy": -0.4},
            {"type": "label", "point": "D", "text": "D", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "E", "text": "E", "dx": 0.15, "dy": 0.4},
            {"type": "label", "point": "F", "text": "F", "dx": 0.3, "dy": -0.4},
        ],
        "labels": ["A", "B", "C", "D", "E", "F"],
    })
}

/// A 字型相似（DE ∥ BC）图纸。
pub(crate) fn similar_a_shape_diagram() -> Value {
    json!({
        "points": {
            "A": [0, 0], "B": [9, 0], "C": [2, 6],
            "D": [3, 0], "E": [1, 3],
        },
        "objects": [
            {"type": "segment", "ends": ["A", "B"]},
            {"type": "segment", "ends": ["B", "C"]},
            {"type": "segment", "ends": ["C", "A"]},
            {"type": "segment", "ends": ["D", "E"], "dashed": true},
            {"type": "label", "point": "A", "text": "A", "dx": -0.35, "dy": 0.4},
            {"type": "label", "point": "B", "text": "B", "dx": 0.15, "dy": 0.4},
            {"type": "label", "point": "C", "text": "C", "dx": 0.3, "dy": -0.4},
            {"type": "label", "point": "D", "text": "D", "dx": 0.15, "dy": 0.4},
            {"type": "label", "point": "E", "text": "E", "dx": -0.5, "dy": 0.2},
        ],
        "labels": ["A", "B", "C", "D", "E"],
    })
}

// ---------- 模板十一：有理数运算 ----------

pub(crate) fn rational_ops(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "rational_ops_addition",
            "计算：$-3 + 5$。",
            "2（异号两数相加，取绝对值较大的符号，用较大的绝对值减较小的绝对值）",
        ),
        Difficulty::Variant => (
            "rational_ops_mixed",
            "计算：$(-2) \\times 3 - 4 \\div 2$。",
            "$-8$（先乘除后加减：$-6 - 2 = -8$）",
        ),
        Difficulty::Advanced => (
            "rational_ops_powers",
            "计算：$-1^2 \\times 2 + (-3)^2 \\div 3$。",
            "1（$-1^2 = -1$，$(-3)^2 = 9$；原式 $= -2 + 3 = 1$）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "有理数运算".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板十二：因式分解 ----------

pub(crate) fn factorization(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "factorization_diff_squares",
            "因式分解：$x^2 - 9$。",
            "$(x + 3)(x - 3)$（平方差公式）",
        ),
        Difficulty::Variant => (
            "factorization_perfect_square",
            "因式分解：$x^2 + 6x + 9$。",
            "$(x + 3)^2$（完全平方公式）",
        ),
        Difficulty::Advanced => (
            "factorization_common_factor",
            "因式分解：$2x^2 - 8$。",
            "$2(x + 2)(x - 2)$（先提公因式 2，再用平方差公式）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "因式分解".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板十三：一般过去时 ----------

pub(crate) fn past_simple(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "past_simple_fill",
            "用所给词的适当形式填空：He ___ (go) to the park yesterday.",
            "went（一般过去时，go 的过去式是不规则变化）",
        ),
        Difficulty::Variant => (
            "past_simple_negative",
            "改为否定句：She watched TV last night.",
            "She didn't watch TV last night.（一般过去时否定用 didn't + 动词原形）",
        ),
        Difficulty::Advanced => (
            "past_simple_question",
            "改为一般疑问句并作肯定回答：They played basketball yesterday.",
            "Did they play basketball yesterday? Yes, they did.（助动词 Did + 动词原形）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "一般过去时".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板十四：现在进行时 ----------

pub(crate) fn present_continuous(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "present_continuous_fill",
            "用所给词的适当形式填空：Look! The boy ___ (swim) in the river.",
            "is swimming（现在进行时：am/is/are + 现在分词）",
        ),
        Difficulty::Variant => (
            "present_continuous_negative",
            "改为否定句：They are playing football now.",
            "They aren't playing football now.（are not 简写 aren't）",
        ),
        Difficulty::Advanced => (
            "present_continuous_listen",
            "用所给词的适当形式填空：Listen! Someone ___ (sing) in the next room.",
            "is singing（Listen! 提示动作正在进行，用现在进行时）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "现在进行时".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}

// ---------- 模板十五：一般将来时 ----------

pub(crate) fn future_simple(difficulty: Difficulty) -> PracticeItem {
    let (template_id, question, answer) = match difficulty {
        Difficulty::Basic => (
            "future_simple_will",
            "用所给词的适当形式填空：We ___ (visit) the museum tomorrow.",
            "will visit（一般将来时：will + 动词原形）",
        ),
        Difficulty::Variant => (
            "future_simple_negative",
            "改为否定句：I will go to Beijing next week.",
            "I won't go to Beijing next week.（will not 简写 won't）",
        ),
        Difficulty::Advanced => (
            "future_simple_going_to",
            "用所给词的适当形式填空：She ___ (buy) a new dress this weekend.",
            "is going to buy（be going to + 动词原形，表示计划打算）",
        ),
        Difficulty::Exam => unreachable!("真题由池内抽取，不走模板"),
    };
    PracticeItem {
        knowledge_point: "一般将来时".into(),
        template_id: template_id.into(),
        difficulty,
        question_text: question.into(),
        answer_spec: answer.into(),
        diagram_spec: None,
        source: None,
    }
}
