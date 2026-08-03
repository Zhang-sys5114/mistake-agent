# 变式出题设计备忘（场景二预研，本期不实现）

> 针对"复盘错题时 Agent 生成变式题"（任务书场景 2：薄弱知识点巩固提升）。几何题是难点：图形与约束互相绑定，模型自由出题容易出现不自洽图形（角度/边长矛盾、无解或多解）。因此方案是**结构化出题 + 内核校验**，而非纯 prompt 自由生成。

## 1. 变式模板库

- 每个知识点维护 3~5 个题型模板，参数槽位化（如"三角形全等：两边及夹角"、"圆与切线：半径/切线长"、"相似：比例系数"）。
- 模板保证题型骨架可解；模型只负责：选模板、填参数（数值/条件强弱/图形朝向）、换背景情境（应用题化）。
- 分层变式：同一模板按参数控制难度——基础（直接套用）/ 同类变式（条件隐藏或逆用）/ 综合拔高（多步组合、辅助线、与其他知识点联动）。高考真题单独入池并标注来源。

## 2. 结构化题目规格（json_schema 强约束，与判分同一模式）

```json
{
  "knowledge_point": "三角形全等判定",
  "template_id": "sss_proof",
  "difficulty": "basic|variant|advanced",
  "question_text": "…",
  "answer_spec": "…（表达式/步骤，供对拍）",
  "diagram_spec": {
    "points": { "A": [0, 0], "B": [4, 0], "C": [2, 3] },
    "objects": [
      {"type": "segment", "ends": ["A", "B"]},
      {"type": "circle", "center": "O", "radius": 3},
      {"type": "parallel", "lines": ["AB", "CD"]}
    ],
    "labels": ["A", "B", "C", "O"]
  }
}
```

- `diagram_spec` 用 GeoGebra 风格坐标/约束列表：机器可校验、UI 可渲染成 SVG（点/线段/圆/标注，可缩放）。
- 题目正文、答案、图形三者同源：都从同一份规格生成，避免"题面与答案对不上"。

## 3. 内核校验（compute::verify 的正式用途）

出题规格生成后交给验算执行端（Pyodide + SymPy）做可解性对拍：

1. 存在性：约束方程组有解（如三边能否构成三角形、圆与线是否相交）。
2. 一致性：按 answer_spec 求出的答案与题目宣称一致。
3. 自洽性：角度和、边长三角不等式、特殊位置关系（切线垂直半径等）。

校验失败 → 换参数重出；连续失败 N 次即停（复用工具护栏语义）。ComputeService 契约已在 services.rs 就位（UnavailableCompute 占位，M4 接 GUI Pyodide）。

## 4. UI 渲染（怎么画图）

**原则：模型不画图，只出图纸规格（diagram_spec）；自研确定性 SVG 渲染器来画。** 渲染是纯数学（两点一线、圆心半径圆、角弧、等长标记、直角标记），不存在"画不出来"的模糊性；图与题面、答案三者同源，不会自相矛盾。不选方案：

- GeoGebra 内嵌引擎：功能全但体积大（MB 级）、离线打包重，对"出静态题目图"是杀鸡用牛刀；
- LLM 直接输出 SVG：无法校验几何自洽，且引入 XSS 面（必须净化），仅适合装饰性插图；
- TikZ/LaTeX 渲染：本地 WebView 无 TeX 发行版，违反本地优先。

**已实现原型**（web/src/lib/geometry.js + components/GeometryFigure.vue，UI 右下「图形」按钮可看示例）：

- 输入 `diagram_spec`：points（命名坐标）+ objects（segment/polygon/circle/right_mark/equal_ticks/angle_arc/label，支持 dashed/color）。
- 输出 SVG：视口自动适配点集边界 + 外扩 margin；直角标记、等长双杠、虚线辅助线、点标签全部确定性生成。
- 安全：渲染器只接受数字坐标与白名单对象类型，输出由我们拼接（不执行模型 HTML），无 XSS 面；未来若支持原始 SVG 导入，一律过 DOMPurify + 白名单。
- 冒烟验证：node 直接调用渲染器生成样例 SVG（viewBox=-1.2 -1.2 8.4 6.4，含三角形/直角标记/等长标记/虚线圆）。

后续扩展：角度弧自动小/大弧判定（已实现）、平行箭头标记、填充多边形（已支持 fill）、按模板参数化的标注密度、图与解析并排。

## 5. 落地时机

场景二实施时按本文档实现 `practice::generate` 工具（practice 插件 + 模板库 + 校验管线）；本期 M6 只覆盖场景一，不实现。
