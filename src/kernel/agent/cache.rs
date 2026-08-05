//! 聊天上下文缓存命中统计（2026-08-04）：
//! 主模型（DeepSeek Responses API）每次回合调用都会携带完整聊天历史，
//! 服务端对相同前缀命中磁盘缓存（input_tokens_details.cached_tokens）。
//! 这里按「会话 + 全局」聚合命中/未命中 tokens，供聊天页展示缓存命中率。
//! 只统计 agent loop 的回合调用（TurnOutcome.usage），不混入守卫/摘要等小调用。

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::{Value, json};

use crate::kernel::agent::session::SessionKey;
use crate::kernel::plugin::services::TokenUsage;

#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheStats {
    pub calls: u64,
    pub hit_tokens: u64,
    pub miss_tokens: u64,
}

impl CacheStats {
    /// 记录一次有 usage 的调用；无任何输入 token 信息时跳过（不算进调用数）。
    pub fn record(&mut self, usage: &TokenUsage) {
        let (hit, miss) = hit_miss(usage);
        if usage.input_tokens.is_none() && usage.cached_tokens.is_none() {
            return;
        }
        self.calls += 1;
        self.hit_tokens += hit;
        self.miss_tokens += miss;
    }

    /// 缓存命中率（0.0 ~ 1.0）；没有可计算样本时返回 None。
    pub fn hit_rate(&self) -> Option<f64> {
        let total = self.hit_tokens + self.miss_tokens;
        if total == 0 {
            None
        } else {
            Some(self.hit_tokens as f64 / total as f64)
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "calls": self.calls,
            "hit_tokens": self.hit_tokens,
            "miss_tokens": self.miss_tokens,
            "hit_rate": self.hit_rate(),
        })
    }
}

fn hit_miss(usage: &TokenUsage) -> (u64, u64) {
    let hit = usage.cached_tokens.unwrap_or(0);
    let miss = usage
        .cache_miss_tokens
        .or_else(|| usage.input_tokens.map(|i| i.saturating_sub(hit)))
        .unwrap_or(0);
    (hit, miss)
}

/// 全局缓存统计（进程生命周期内累计；不落盘，重启清零）。
#[derive(Default)]
pub struct CacheTracker {
    main_global: Mutex<CacheStats>,
    sessions: Mutex<HashMap<String, CacheStats>>,
}

impl CacheTracker {
    /// 记录一次主模型回合调用（按会话 + 全局累计）。
    pub fn record_main(&self, key: &SessionKey, usage: &TokenUsage) {
        self.main_global
            .lock()
            .expect("cache poisoned")
            .record(usage);
        self.sessions
            .lock()
            .expect("cache poisoned")
            .entry(key.to_string())
            .or_default()
            .record(usage);
    }

    /// 快照（RPC get_cache_stats）：主模型全局 + 各会话明细 + 当前活动会话。
    pub fn snapshot(&self, active_key: Option<SessionKey>) -> Value {
        let main = self.main_global.lock().expect("cache poisoned");
        let sessions = self.sessions.lock().expect("cache poisoned");
        let mut session_list: Vec<Value> = sessions
            .iter()
            .map(|(k, v)| {
                json!({
                    "key": k,
                    "calls": v.calls,
                    "hit_tokens": v.hit_tokens,
                    "miss_tokens": v.miss_tokens,
                    "hit_rate": v.hit_rate(),
                })
            })
            .collect();
        session_list.sort_by_key(|v| std::cmp::Reverse(v["calls"].as_u64().unwrap_or(0)));
        json!({
            "main": main.as_json(),
            "sessions": session_list,
            "active_key": active_key.map(|k| k.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_rate_math() {
        let mut s = CacheStats::default();
        assert_eq!(s.hit_rate(), None);
        s.record(&TokenUsage {
            input_tokens: Some(100),
            cached_tokens: Some(80),
            cache_miss_tokens: Some(20),
            ..Default::default()
        });
        s.record(&TokenUsage {
            input_tokens: Some(50),
            cached_tokens: Some(10),
            cache_miss_tokens: Some(40),
            ..Default::default()
        });
        assert_eq!(s.calls, 2);
        assert_eq!(s.hit_tokens, 90);
        assert_eq!(s.miss_tokens, 60);
        let rate = s.hit_rate().unwrap();
        assert!((rate - 0.6).abs() < 1e-9);
    }

    #[test]
    fn record_skips_empty_usage() {
        let mut s = CacheStats::default();
        s.record(&TokenUsage::default());
        assert_eq!(s.calls, 0);
        assert_eq!(s.hit_rate(), None);
    }

    #[test]
    fn tracker_snapshot_includes_sessions() {
        let t = CacheTracker::default();
        let key = SessionKey::new();
        t.record_main(
            &key,
            &TokenUsage {
                input_tokens: Some(10),
                cached_tokens: Some(6),
                cache_miss_tokens: Some(4),
                ..Default::default()
            },
        );
        let snap = t.snapshot(Some(key));
        assert_eq!(snap["main"]["calls"], 1);
        assert_eq!(snap["sessions"][0]["key"], key.to_string());
        assert_eq!(snap["active_key"], key.to_string());
    }
}
