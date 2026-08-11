//! 代数模板：绝对值/方程/函数/有理数/因式分解。

use super::*;

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
