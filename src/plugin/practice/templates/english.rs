//! 英语模板：时态（三单/过去/进行/将来）。

use super::*;

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
