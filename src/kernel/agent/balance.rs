//! 余额查询（账户外部 API 轻量封装，2026-08-04 双提供商实测通过）：
//! - DeepSeek：`GET {base}/user/balance`，返回 `balance_infos`（可能多币种，取第一条）；
//! - SiliconFlow：`GET {base}/user/info`，返回 `data.balance`（赠送余额）、
//!   `chargeBalance`（充值余额，实际可用）、`totalBalance`（总额）。
//!
//! 只读账户信息，绝不把 api_key 放进返回值或审计；未配置时给出结构化占位。

use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::{Value, json};

use crate::kernel::settings::Settings;

#[derive(Debug, Clone, Serialize)]
pub struct BalanceReport {
    /// DeepSeek（主模型）。
    pub main: ProviderBalance,
    /// SiliconFlow（视觉模型）。
    pub vision: ProviderBalance,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderBalance {
    /// 该服务是否已配置 api_key。
    pub configured: bool,
    /// 查询是否成功；未配置时 ok=false 且 error=None（前端按 configured 展示）。
    pub ok: bool,
    pub error: Option<String>,
    /// 提炼后的展示字段（各提供商结构不同，字段名保持原文方便前端直接取用）。
    pub data: Option<Value>,
}

/// 查询两个提供商余额（各一次真实 HTTP 请求；5xx 临时错误重试一次）。
pub async fn check_balance(settings: &Settings) -> BalanceReport {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        // 与模型服务一致：无 IPv6 环境下强制走 IPv4，避免解析到 v6 后连接立即失败。
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .build()
        .expect("reqwest client 构建失败");

    BalanceReport {
        main: provider_balance(
            &client,
            &settings.main_model,
            deepseek_url(&settings.main_model.api_url),
            deepseek_display,
        )
        .await,
        vision: provider_balance(
            &client,
            &settings.vision_model,
            siliconflow_url(&settings.vision_model.api_url),
            siliconflow_display,
        )
        .await,
    }
}

async fn provider_balance(
    client: &Client,
    cfg: &crate::kernel::settings::ModelConfig,
    url: String,
    display: fn(&Value) -> Value,
) -> ProviderBalance {
    let key = cfg.api_key.trim();
    if key.is_empty() {
        return ProviderBalance {
            configured: false,
            ok: false,
            error: None,
            data: None,
        };
    }
    match fetch_json(client, &url, key).await {
        Ok(raw) => ProviderBalance {
            configured: true,
            ok: true,
            error: None,
            data: Some(display(&raw)),
        },
        Err(e) => ProviderBalance {
            configured: true,
            ok: false,
            error: Some(e),
            data: None,
        },
    }
}

/// GET + Bearer 鉴权；5xx（含 503）重试一次，其余错误直接返回。
async fn fetch_json(client: &Client, url: &str, key: &str) -> Result<Value, String> {
    let mut last_error: Option<String> = None;
    for attempt in 0..2 {
        let resp = client
            .get(url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| format!("网络请求失败：{e}"));
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_error = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
                continue;
            }
        };
        let status = resp.status();
        if status.is_success() {
            return resp
                .json::<Value>()
                .await
                .map_err(|e| format!("响应解析失败：{e}"));
        }
        let body = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(160)
            .collect::<String>();
        last_error = Some(format!("HTTP {}：{}", status.as_u16(), body));
        if status.is_server_error() {
            tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
            continue;
        }
        break;
    }
    Err(last_error.unwrap_or_else(|| "余额查询失败".into()))
}

/// DeepSeek 余额端点：base 与模型端点同源；兼容带 /v1 的写法，取根路径。
fn deepseek_url(api_url: &str) -> String {
    let base = api_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/user/balance")
}

/// SiliconFlow 余额端点：OpenAI 兼容基址带 /v1；用户可能只配了裸域，自动补全。
fn siliconflow_url(api_url: &str) -> String {
    let base = api_url.trim_end_matches('/');
    let base = if base.ends_with("/v1") {
        base.to_string()
    } else {
        format!("{base}/v1")
    };
    format!("{base}/user/info")
}

/// 提炼 DeepSeek balance_infos（多币种取第一条）。
fn deepseek_display(raw: &Value) -> Value {
    let info = raw
        .get("balance_infos")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .cloned()
        .unwrap_or_default();
    json!({
        "is_available": raw.get("is_available").cloned().unwrap_or(Value::Null),
        "currency": info.get("currency").cloned().unwrap_or(Value::Null),
        "total_balance": info.get("total_balance").cloned().unwrap_or(Value::Null),
        "granted_balance": info.get("granted_balance").cloned().unwrap_or(Value::Null),
        "topped_up_balance": info.get("topped_up_balance").cloned().unwrap_or(Value::Null),
    })
}

/// 提炼 SiliconFlow data 字段。
fn siliconflow_display(raw: &Value) -> Value {
    let data = raw.get("data").cloned().unwrap_or_default();
    json!({
        "name": data.get("name").cloned().unwrap_or(Value::Null),
        "status": data.get("status").cloned().unwrap_or(Value::Null),
        "balance": data.get("balance").cloned().unwrap_or(Value::Null),
        "charge_balance": data.get("chargeBalance").cloned().unwrap_or(Value::Null),
        "total_balance": data.get("totalBalance").cloned().unwrap_or(Value::Null),
    })
}
