# Prompt 设计文档

> Prompt 是 Agent 行为的第一生产力，本文件集中维护并留痕迭代（任务书交付物之一）。代码位置：src/kernel/prompt.rs；改动本文件后同步更新代码，反之亦然。

## 设计原则

1. **角色先行**：每条 prompt 先立角色（面向中学生的本地错题学习助手），再给流程，再给表达规范。
2. **工具即流程**：把业务工作流（上传 → OCR → 判分 → 归档 → 讲解）写进系统提示，让模型知道何时、以什么顺序调工具。
3. **失败可恢复**：明确区分"换参数重试"与"系统性错误直接告知"，配合内核护栏（同码连续失败 3 次即停）。
4. **不泄露思维链**：reasoning 不进学生可见内容（UI 侧默认折叠仅为调试/透明）。
5. **输出结构化**：判分用 json_schema 强约束（服务端强制数组结构），不靠"请输出 JSON"的软约束。
6. **只提取不判分**：视觉模型严格只做 OCR（用户明确要求），判分交给主模型。

## Prompt 清单

### 1. Agent 系统提示（agent_system_prompt）

注入位置：主模型每个请求的消息头（src/kernel/loop_mod.rs 组装 ModelRequest 时注入，不落消息树）。内容见 src/kernel/prompt.rs，要点：

- 角色与受众（中学生、中文、耐心）。
- 工具流程：`grading::upload`（OCR→判分→归档→讲解）、`grading::list`（错题本查询）。
- 不引导用户输入图片/PDF 路径：作业文件由界面「选择作业文件」按钮上传并自动暂存，模型只用消息里给出的暂存路径。
- 失败处理分级（可重试一次 / 系统性错误直接告知）。
- 表达规范（数学记号、不展示 reasoning、敏感话题引导求助）。
- **LaTeX 增强渲染**：数学/物理/化学等富文本一律用 `$...$` / `$$...$$` 标记（`\frac`、`\sqrt`、`\mathrm`、`\vec`、`pmatrix`），前端 KaTeX 渲染。
- 本地运行环境说明。

### 2. OCR 提示（ocr_prompt）

视觉模型（Qwen/Qwen3-VL-32B-Instruct）使用：逐字转写题目与作答、保留题号与数学符号、**不解题不评判**。

### 3. 判分系统提示（grading_system_prompt）

主模型使用，配合 `text.format = json_schema`（内联扁平数组 schema，src/plugin/grading.rs）：
逐题输出 number/question/student_answer/correct/score/total/knowledge_point/analysis/subject/reference_answer；
`subject` 为学科（数学/英语/物理/化学/生物/语文等，无法判断填"未分类"），`reference_answer` 为该题参考答案（可 null）；
强制数组包裹。

### 4. 会话切换决策提示（turn_decider_prompt）— M2 落地，ADR-0030/0032

生产实现 = 主模型 + turn_decider_prompt 独立调用，输出严格 JSON：continue / update_goal / start_new 三动作；
输入含 new_text（新消息，可能为 null）——**new_text 非空 = 新消息到达，先判断是否切换上下文再回答**；
new_text 为 null = 回合结束判断目标是否完成。存疑即 continue，start_new 仅当目标明显无关；
解析失败/调用失败时按 continue 兜底（存疑即继续）。

### 5. 压缩/交接摘要提示（summarize_prompt）— M2 落地

生产实现 = 主模型 + summarize_prompt 生成：保留错题 id、知识点、未完成事项、结论；≤300 字；
用于会话交接摘要（旧会话归档）与上下文压缩。

## 迭代记录

| 日期 | 变更 | 原因/结果 |
|---|---|---|
| 2026-08-04 | OCR 提示从"识别+解题"改为"只提取" | 用户明确：视觉模型不回答问题，只提取内容（实测已通） |
| 2026-08-04 | 判分从 json_object 改为 json_schema 内联数组 | json_object 曾返回单对象；DeepSeek 端不解析 $defs/$ref，内联扁平 schema 后服务端强制数组（实测已通） |
| 2026-08-04 | 系统提示注入 loop（每请求注入，不落盘） | 无状态 Responses API 需要每次带全量上下文；消息树保持"用户可见对话"纯净 |
| 2026-08-04 | 判分新增 subject / reference_answer 字段 | 归档错题绑定真实学科、保留参考答案，支撑后续按学科过滤与变式出题（技术债清偿） |
| 2026-08-04 | 守卫/摘要从 stub 升级为模型生产实现 | 会话切换决策与交接摘要改由主模型生成，stub 关键词版退役（保留接口兼容） |
| 2026-08-04 | 系统提示要求富文本输出 LaTeX 标记 | 数学/物理/化学公式经前端 KaTeX 增强渲染，避免 Unicode 伪符号 |
| 2026-08-04 | 判分 prompt 补 LaTeX 规范 + 词形/时态填空判定 | 公式一律 $...$ 标记；词性/时态转换正确即判对（修 sunny 误判观察项） |

## 评估记录（真实 API）

- 三套样例端到端（samples/，2026-08-04 实测）：
  - sample1 真实照片（线代填空）：OCR 准确转写（β₁=(1,0,1)ᵀ 等）；判分正确识别"未作答"、绑定"向量组的线性相关性判断"、给出错因。
  - sample2 合成数学卷（3 题）：2 对 1 错，\|-3\|=-3 被判错（正确）。
  - sample3 合成英语卷（3 题）：1 对 2 错；其中一道本应正确（sunny）被判错——判分 prompt 需针对"词形/时态填空"补充判定说明（观察项，记录于 docs/testing.md）。
- 待补：真实手写作答照片、多页 PDF、多学科人工评估。
