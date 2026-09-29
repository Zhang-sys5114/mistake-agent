//! 限额裁决规则（ADR-0047 决策 6）。
//!
//! **纯函数**：吃套餐限额、权益状态、三窗口已用量，吐出裁决结果。
//! 之所以把规则抽出来，是因为并发正确性由数据库事务（advisory lock）保证，
//! 而"什么算超额"这件事必须能被单测钉死——两者分开，各管一件事。

use super::model::{Entitlement, Plan, QuotaDecision, QuotaDenial, QuotaGrant, WindowUsage};

/// 判定这次请求能不能放行。`windows` 由调用方在同一事务内读取（保证与预留一致）。
pub fn decide(plan: &Plan, entitlement: &Entitlement, windows: WindowUsage) -> QuotaDecision {
    // 体验包：先看一次性总次数（它比窗口更早用尽，提示也更明确）
    if let Some(total) = entitlement.total_uses
        && entitlement.used_uses >= total
    {
        return QuotaDecision::Denied(QuotaDenial::UsesExhausted {
            total,
            used: entitlement.used_uses,
        });
    }

    // 三滑动窗口：逐个检查，报出最贴合用户感知的那一个
    if let Some(limit) = plan.limit_5h
        && windows.used_5h >= i64::from(limit)
    {
        return QuotaDecision::Denied(QuotaDenial::Window5h {
            limit,
            used: windows.used_5h,
        });
    }
    if let Some(limit) = plan.limit_week
        && windows.used_week >= i64::from(limit)
    {
        return QuotaDecision::Denied(QuotaDenial::WindowWeek {
            limit,
            used: windows.used_week,
        });
    }
    if let Some(limit) = plan.limit_month
        && windows.used_month >= i64::from(limit)
    {
        return QuotaDecision::Denied(QuotaDenial::WindowMonth {
            limit,
            used: windows.used_month,
        });
    }

    QuotaDecision::Allowed(QuotaGrant {
        entitlement: entitlement.clone(),
        plan: plan.clone(),
        windows,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;
    use crate::billing::model::{EntitlementSource, PlanKind};

    fn plan(limit_5h: Option<i32>, limit_week: Option<i32>, limit_month: Option<i32>) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            code: "monthly_lite".into(),
            name: "月卡 Lite".into(),
            kind: PlanKind::Monthly,
            total_uses: None,
            limit_5h,
            limit_week,
            limit_month,
        }
    }

    fn entitlement(total_uses: Option<i32>, used_uses: i32) -> Entitlement {
        Entitlement {
            id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            source: EntitlementSource::Redeem,
            expires_at: Utc::now() + chrono::Duration::days(30),
            total_uses,
            used_uses,
        }
    }

    #[test]
    fn allows_when_all_windows_have_room() {
        let decision = decide(
            &plan(Some(15), Some(100), Some(300)),
            &entitlement(None, 0),
            WindowUsage {
                used_5h: 14,
                used_week: 99,
                used_month: 299,
            },
        );
        assert!(matches!(decision, QuotaDecision::Allowed(_)));
    }

    #[test]
    fn denies_at_the_window_boundary_inclusively() {
        // 边界含端：已用 == 上限即拒绝（"15 次"的套餐只能成功 15 次）
        let cases = [
            (
                WindowUsage {
                    used_5h: 15,
                    used_week: 0,
                    used_month: 0,
                },
                "window_5h_exceeded",
            ),
            (
                WindowUsage {
                    used_5h: 0,
                    used_week: 100,
                    used_month: 0,
                },
                "window_week_exceeded",
            ),
            (
                WindowUsage {
                    used_5h: 0,
                    used_week: 0,
                    used_month: 300,
                },
                "window_month_exceeded",
            ),
        ];
        for (windows, expected) in cases {
            let decision = decide(
                &plan(Some(15), Some(100), Some(300)),
                &entitlement(None, 0),
                windows,
            );
            match decision {
                QuotaDecision::Denied(denial) => assert_eq!(denial.code(), expected),
                QuotaDecision::Allowed(_) => panic!("{expected} 应被拒绝"),
            }
        }
    }

    #[test]
    fn null_limit_means_unlimited() {
        // 体验包：窗口全不限制，只受总次数约束
        let decision = decide(
            &plan(None, None, None),
            &entitlement(Some(10), 3),
            WindowUsage {
                used_5h: 999,
                used_week: 9999,
                used_month: 99999,
            },
        );
        assert!(matches!(decision, QuotaDecision::Allowed(_)));
    }

    #[test]
    fn experience_package_denies_when_exhausted() {
        let decision = decide(
            &plan(None, None, None),
            &entitlement(Some(10), 10),
            WindowUsage::default(),
        );
        match decision {
            QuotaDecision::Denied(denial) => {
                assert_eq!(denial.code(), "uses_exhausted");
                assert!(denial.message().contains("10"));
            }
            QuotaDecision::Allowed(_) => panic!("次数用尽应被拒绝"),
        }
    }

    #[test]
    fn total_uses_denial_takes_precedence_over_windows() {
        // 两种都不满足时，报"体验包用完"比报窗口更贴近用户能做的事（去兑换月卡）
        let decision = decide(
            &plan(Some(1), Some(1), Some(1)),
            &entitlement(Some(5), 5),
            WindowUsage {
                used_5h: 1,
                used_week: 1,
                used_month: 1,
            },
        );
        match decision {
            QuotaDecision::Denied(denial) => assert_eq!(denial.code(), "uses_exhausted"),
            QuotaDecision::Allowed(_) => panic!("应被拒绝"),
        }
    }

    #[test]
    fn denial_messages_are_user_facing() {
        // 面向中学生与家长：不出现 token / 窗口 这类术语
        let denials = [
            QuotaDenial::NoEntitlement,
            QuotaDenial::UsesExhausted {
                total: 10,
                used: 10,
            },
            QuotaDenial::Window5h {
                limit: 15,
                used: 15,
            },
            QuotaDenial::WindowWeek {
                limit: 100,
                used: 100,
            },
            QuotaDenial::WindowMonth {
                limit: 300,
                used: 300,
            },
        ];
        for denial in denials {
            let message = denial.message();
            assert!(!message.is_empty());
            assert!(!message.contains("token"), "{message}");
            assert!(!message.contains("窗口"), "{message}");
        }
    }
}
