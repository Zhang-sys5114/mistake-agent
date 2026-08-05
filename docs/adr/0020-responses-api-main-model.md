# 主模型经 DeepSeek Responses API 接入（含工具 wire name 映射）

> 更新（2026-08-05）：wire name 映射由 `::`→`_` 改为 `::`→`__`（双下划线）——`a::b_c` → `a__b_c`、`a_b::c` → `a_b__c`，插件内部的下划线不再与分隔符撞名；`a::b__c` vs `a__b::c` 这类双下划线病态组合仍由注册期全局唯一校验兜底。
> 更新（2026-08-05）：thinking 模式回放校验实测要求**每个 `function_call` 前都紧跟一条 reasoning item**（模型输出可一个 reasoning 带多个并行调用，回放时第二起调用会被判"未回传"而 400）。`messages_to_responses_input` 按调用复制该轮 reasoning（同 id 同文本）；若仍被拒，`ResponsesModelService` 兜底重试一次——剥离全部 reasoning + `reasoning.effort=none`（丢思考连续性但不让回合失败）。真实 API 四种历史结构复验通过。

DeepSeek 官方已支持 OpenAI Responses API 格式（POST /responses，base_url 与 Chat Completions 相同），目前仅 deepseek-v4-flash 可用（v4-pro 计划 2026 年 8 月初支持）。主模型适配器改用 Responses API：它为 agent 场景提供语义化流式事件（output_text.delta / function_call_arguments.delta / reasoning_text.delta，结束事件 response.completed/incomplete/failed）与思考模式；API 无状态，kernel 每回合发送全量历史，与消息树设计一致。视觉模型（qwen3-VL/SiliconFlow）不迁移：Responses API 不支持图片输入，视觉模型继续走 Chat Completions；Ollama 等不兼容端点经 settings 的 transport 配置回退到 Chat Completions。

工具名约束是迁移的直接后果：Responses API 要求 function 名匹配 ^[a-zA-Z0-9_-]+$，内部规范名 namespace::tool 含 :: 无法直接发送。因此内部名保持不变，适配器边界生成 wire name（:: → __），注册时校验 wire name 全局唯一保证一一对应，模型回包经 dispatch 映射回内部全名。另注意：parallel_tool_calls 恒开启（参数被忽略），模型一轮可能输出多个 function_call，loop 按 ADR-0010 串行执行；thinking 模式下 temperature/top_p 不生效，max_output_tokens 含推理 token；上下文缓存由服务端自动管理。

来源：DeepSeek 官方文档（https://api-docs.deepseek.com/guides/responses_api/ 、https://api-docs.deepseek.com/zh-cn/guides/responses_api/ 、https://api-docs.deepseek.com/api/create-response/ 、https://api-docs.deepseek.com/quick_start/pricing/），2026-08-04 核对中英文版一致。
