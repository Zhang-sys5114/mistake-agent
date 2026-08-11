//! 几何模板（含 diagram_spec）：三角形全等/勾股/相似。

use super::*;

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
