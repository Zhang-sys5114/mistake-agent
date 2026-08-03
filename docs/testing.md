# 测试文档

## 1. 测试策略

- **单元测试**：`cargo test`，覆盖注册表校验、dispatch、session 调度、storage、memory、model 解析（不依赖网络）。
- **真实 API 集成测试**：`cargo test --test live_api -- --ignored --nocapture`，直接接 DeepSeek/SiliconFlow（无 key 自动跳过）。
- **样例端到端**：`samples/` 三套作业图片逐一走 上传→OCR→判分→归档 全链路。

## 2. 用例与结果（2026-08-04 实测）

### 单元测试：14 项全过

| 模块 | 覆盖点 |
|---|---|
| registry | namespace 撞名、wire 撞名、requires 不可满足、懒注册 |
| dispatch | （同 registry 测试中覆盖注册链路） |
| session | 首消息建会话、空闲超时开新会话、守卫关键词切换、InterruptBus |
| storage | 错题 CRUD、会话追加/归档 |
| memory | 目录浏览、子树删除、路径校验（绝对/../空段） |
| model | SSE 解析、ToolCall 展开为 function_call + output |

### 真实 API 链路

| 用例 | 结果 |
|---|---|
| hello 回合（send_user_message → Responses API 流式） | ✅ 通过（多次实测，事件数 26~120） |
| 三套样例批改（grading::upload） | ✅ 通过 |

| 样例 | 类型 | 题数 | 对 | 错 | 归档 | 备注 |
|---|---|---|---|---|---|---|
| sample1_linalg_real.png | 真实照片（线代填空） | 1 | 0 | 1 | 1 | 未作答判错，知识点"向量组的线性相关性判断" |
| sample2_math_synthetic.png | 合成数学卷 | 3 | 2 | 1 | 1 | \|-3\|=-3 判错正确 |
| sample3_english_synthetic.png | 合成英语卷 | 3 | 1 | 2 | 2 | 见观察项 #1 |

### 门禁

`cargo fmt --check` ✅ ｜ `cargo clippy --all-targets -- -D warnings` ✅ ｜ `cargo test` ✅ ｜ GUI 冒烟（Wayland 下启动 8s 无崩溃）✅

## 3. Bug / 观察列表

| # | 现象 | 根因 | 状态 |
|---|---|---|---|
| 1 | 英语合成卷 3 题中 1 道"本应正确"被判错 | 模型判分歧义（"sunny" vs "sun" 时态/词性判断），非链路故障；属模型行为，后续用判分 prompt 与样例校准 | 观察中 |
| 2 | sidecar 管道 EOF 后回合被取消（DNS task cancelled） | 回合任务独立于 JoinSet，main 返回触发运行时关闭 | 已修：EOF 后轮询 `is_idle` 再退出 |
| 3 | Tauri Channel<String> 交付字符串，UI 当对象用导致事件全丢 | 桥接类型不匹配 | 已修：JS 侧 JSON.parse |
| 4 | 无 IPv6 环境 reqwest 连接失败 | 解析到 v6 后无回退 | 已修：客户端强制 IPv4 本地地址 |
| 5 | flexi_logger 重复初始化报错 | 全局 logger 只能 init 一次 | 已修：OnceLock 幂等 |
| 6 | DeepSeek json_schema 返回 schema 原文 | schema 含 $defs/$ref 不被解析 | 已修：内联扁平数组 schema |
| 7 | tokio stdout 写管道丢失 | 环境差异 | 已修：帧写入改同步 stdout（协议通道，绝不含日志） |

## 4. 成本观察（真实调用）

- 视觉 OCR（Qwen3-VL-32B）：线代题图 prompt 267 / completion 815 tokens。
- 主模型判分（deepseek-v4-flash，thinking=none）：每题约 1~4 秒，json_schema 强制结构化。

## 5. 安全测试

- XSS：Markdown 输入含 `<script>`、事件属性、`javascript:` 链接时，DOMPurify 净化后不执行（渲染层唯一 v-html 入口，其余文本均走 Vue 插值自动转义）。CSP：`script-src 'self'`，无内联脚本。
- XXE：当前全栈无 XML 解析（无 SAX/DOM/外部实体处理）；PDF 用 lopdf 二进制解析；Markdown/HTML 渲染不解析 XML。若未来引入 SVG/XML 上传，须先过 DOMPurify 或专用解析器白名单。

## 5. 待补（记录在案）

- 至少 3 套**真实手写作业照片**端到端（当前 1 真实 + 2 合成）。
- 多页 PDF（含图片页）渲染 OCR。
- 判分质量评估：多学科样例人工核对（任务书要求 Prompt 评估报告）。
