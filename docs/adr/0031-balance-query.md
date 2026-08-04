# 0031 — 账户余额查询（check_balance RPC）

日期：2026-08-04
状态：已采纳

## 背景

本地 Agent 的两条模型链路（DeepSeek 主模型、SiliconFlow 视觉模型）都可能因余额不足而静默失败。用户在设置页希望一眼看到两个账户的余额，避免"模型没反应"时只能去官网查。

## 决策

1. **新增 `check_balance` RPC**（`Method::CheckBalance`，wire 名 `check_balance`），只读、无参数；由 `src/kernel/balance.rs` 实现，不占用工具注册表（不向模型暴露）。
2. **端点**：
   - DeepSeek：`GET {api_url 去 /v1}/user/balance`，`Authorization: Bearer <key>`；取 `balance_infos[0]` 的 currency/total_balance/granted_balance/topped_up_balance，配 `is_available`。
   - SiliconFlow：`GET {api_url 补齐 /v1}/user/info`，`Authorization: Bearer <key>`；取 `data` 的 name/status/balance（赠送余额）/chargeBalance（充值余额，实际可用）/totalBalance（总额）。
3. **密钥策略延续 ADR-0015/0027**：直接读 settings.json 中的真实 key（只用于 HTTP 头），返回值、审计日志、错误信息一律不含 key。
4. **容错**：未配置 key → `configured=false, ok=false, error=null`（前端显示"未配置"）；5xx（含 503）与网络错误重试一次（500ms/1s 退避），4xx 直接失败并附状态码与截断响应体。
5. **审计**：新增 `AuditRecord::BalanceChecked { main_ok, vision_ok }`，只记成功与否，不记金额。
6. **前端**：设置页顶部新增"账户余额"卡片，挂载与保存设置后自动查询，可手动刷新；DeepSeek 显示总余额与可用状态，SiliconFlow 显示可用/充值/总额。

## 后果

- 设置页新增一次只读外部调用（约 <1s），失败不影响设置页其他功能。
- 余额数据不落盘、不进会话，只作即时展示；避免敏感账户信息进入消息树/审计。
- SiliconFlow 展示以 `chargeBalance`（充值余额）为主，赠送余额与总额为辅——用户已确认
  `balance` 只是赠送余额，真正可消耗的是充值余额。
