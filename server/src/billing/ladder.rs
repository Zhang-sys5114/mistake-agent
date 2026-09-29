//! 阶梯扣次（ADR-0047 决策 6）：把一次请求折算成对外的「次数」。
//!
//! 按次数售卖的最大风险是单次上下文长度无上限——一个 80k token 的回合与一个 3k 的回合
//! 都算「1 次」。阶梯按 `input + output` 总量分档：跨过一个阈值就多扣一次。
//!
//! 纯函数：不读配置、不碰数据库，只吃 token 数与阈值表——边界行为因此能被测试钉死，
//! 而不是散在转发链路里靠肉眼确认。

/// 计费口径的 token 总量。缓存命中部分**已包含在输入里**（`input_tokens` 是完整输入，
/// 命中数只是其明细），所以不重复相加。缺失（上游错误、中途中断）按 0 处理。
pub fn total_tokens(input_tokens: Option<u64>, output_tokens: Option<u64>) -> u64 {
    input_tokens
        .unwrap_or(0)
        .saturating_add(output_tokens.unwrap_or(0))
}

/// 计算一次请求应扣的次数，恒 ≥ 1（成功响应至少算一次）。
///
/// `ladder` 为严格递增的 token 阈值表（配置层已校验）：总量超过几个阈值就多扣几次。
/// 空表表示不设阶梯，任何请求都只扣 1 次。
pub fn billed_uses(total_tokens: u64, ladder: &[u64]) -> u32 {
    let crossed = ladder
        .iter()
        .filter(|&&threshold| total_tokens > threshold)
        .count();
    // 阈值表来自配置（长度极小），不可能溢出 u32；用饱和加法保证任何输入都不 panic
    u32::try_from(crossed).unwrap_or(u32::MAX).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LADDER: [u64; 2] = [32 * 1024, 64 * 1024];

    #[test]
    fn tier_boundaries_are_inclusive_at_the_threshold() {
        // 契约（ADR-0047 决策 6）：≤32k 扣 1、≤64k 扣 2、>64k 扣 3
        for (tokens, expected) in [
            (0, 1),
            (1, 1),
            (32 * 1024, 1),
            (32 * 1024 + 1, 2),
            (64 * 1024, 2),
            (64 * 1024 + 1, 3),
            (200 * 1024, 3),
        ] {
            assert_eq!(billed_uses(tokens, &LADDER), expected, "tokens={tokens}");
        }
    }

    #[test]
    fn empty_ladder_never_multiplies() {
        for tokens in [0, 1, 10_000_000] {
            assert_eq!(billed_uses(tokens, &[]), 1);
        }
    }

    #[test]
    fn ladder_is_monotonic() {
        // 阈值表可热改（配置/后续 DB），单调性是最起码的不变量
        let mut previous = 0;
        for tokens in (0..=100 * 1024).step_by(1024) {
            let uses = billed_uses(tokens, &LADDER);
            assert!(uses >= previous, "tokens={tokens} 处出现回退");
            previous = uses;
        }
    }

    #[test]
    fn total_tokens_treats_missing_values_as_zero() {
        assert_eq!(total_tokens(Some(120), Some(8)), 128);
        assert_eq!(total_tokens(None, Some(8)), 8);
        assert_eq!(total_tokens(Some(120), None), 120);
        assert_eq!(total_tokens(None, None), 0);
        // 极端输入不 panic
        assert_eq!(total_tokens(Some(u64::MAX), Some(1)), u64::MAX);
    }
}
