// 小问分段：模型写给错题本的 reference_answer / analysis 常常是没有任何换行的
// 一整段（例如「…故椭圆标准方程为 $x^2/9+y^2=1$。（2）（i）$A(0,-1)$，…」），
// marked 只会渲染出一个 <p>，观感很差。这里按小问标记切成独立段落，
// 交给 SubQuestionText 用左竖线缩进块分行渲染。
//
// 判定规则（宁缺勿滥 —— 切错比不切更难看）：
//   1. 先按换行切块，再对每块做小问切分，这样「模型已自己分段」的新数据
//      与「一整段」的历史数据渲染结果一致；
//   2. 标记必须落在句首（紧跟 。；：！？ 或位于块首）才算小问起点，
//      于是「第（1）问」这类正文引用、「（2）（i）」这种连写都不会被误切；
//   3. $...$ 数学段与 ``` 代码块内的标记一律跳过（公式里括号极多）。
//
// 返回段落数组；识别不出分段时返回 null，调用方退回整段渲染。

// （1）(1)（i）(ii)①-⑳ —— 全角/半角括号、大小写罗马数字、带圈数字
const MARKER = /^(?:（\d{1,2}）|\(\d{1,2}\)|（[ivxIVX]{1,4}）|\([ivxIVX]{1,4}\)|[①-⑳])/;

// 句末标点：只有紧跟在它们之后的标记才算小问起点
const LEAD = new Set(["。", "；", "：", "！", "？", "…"]);

/** 单块文本内的小问切分；无标记可切时返回原文本单元素数组。 */
function cutByMarkers(block) {
  const cuts = [];
  let inMath = false;
  let inCode = false;

  for (let i = 0; i < block.length; i += 1) {
    // ``` 代码块（SMILES）：块内内容原样跳过
    if (!inMath && block.startsWith("```", i)) {
      inCode = !inCode;
      i += 2;
      continue;
    }
    if (inCode) continue;

    // $ 开关；\$ 是 LaTeX 转义的字面美元号，不算定界符
    if (block[i] === "$" && block[i - 1] !== "\\") {
      inMath = !inMath;
      continue;
    }
    if (inMath) continue;

    if (i !== 0 && !LEAD.has(block[i - 1])) continue;
    if (MARKER.test(block.slice(i))) cuts.push(i);
  }

  if (cuts.length === 0) return [block];

  // 首个标记之前的内容（如错因分析的总述）单独成段，不能丢
  const bounds = cuts[0] === 0 ? cuts : [0, ...cuts];
  const parts = [];
  for (let k = 0; k < bounds.length; k += 1) {
    const body = block.slice(bounds[k], bounds[k + 1] ?? block.length).trim();
    if (body) parts.push(body);
  }
  return parts;
}

export function splitSubQuestions(text) {
  const src = (text ?? "").trim();
  if (!src) return null;

  const parts = [];
  for (const line of src.split(/\n+/)) {
    const block = line.trim();
    if (block) parts.push(...cutByMarkers(block));
  }
  return parts.length >= 2 ? parts : null;
}
