# 0033 — 聊天上下文缓存命中率

日期：2026-08-04
状态：已采纳

## 背景

主模型（DeepSeek Responses API）每回合都携带完整聊天历史，服务端对相同前缀命中磁盘缓存，
费用远低于未命中。用户希望在聊天页直接看到缓存命中率，及时感知"上下文复用"的性价比。

## 决策

1. **usage 解析**：
   - Responses API：`usage.input_tokens_details.cached_tokens` = 命中；
     未命中 = `input_tokens - cached_tokens`（input_tokens 含命中部分）；
   - Chat Completions（SiliconFlow）：`prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`。
   `TokenUsage` 新增 `cache_miss_tokens` 字段。
2. **统计口径**：只统计 agent loop 的回合调用（`TurnOutcome.usage`，回合内多次模型流累加），
   不混入守卫/摘要等小调用；按「当前会话 + 全局」聚合，进程生命周期累计，不落盘。
3. **RPC**：新增 `get_cache_stats`，返回主模型全局统计、各会话明细、当前活动会话 key；
   命中率 = 命中 / (命中 + 未命中)，无样本时为 null。
4. **前端**：聊天页右上角显示"上下文缓存命中 xx.x%"胶囊，悬停查看本会话与累计明细，
   点击刷新；回合 usage 落盘后由后端实时推送 `cache_stats_updated` 事件（含最新快照），
   前端收到即更新——保证显示的就是本轮刚算出来的数，不依赖前端回查（可能读到旧值）。

## 后果

- 缓存命中率成为聊天页可见指标（实测：双回合后命中率 97.3%，命中 4864 / 未命中 190 tokens）。
- 统计只存内存，重启清零；不写审计、不进消息树，无隐私负担。
