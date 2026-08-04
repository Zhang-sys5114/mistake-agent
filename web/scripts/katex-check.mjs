// LaTeX 渲染链路自检：KaTeX 核心 + marked-katex-extension。
// 运行：cd web && node scripts/katex-check.mjs

import katex from "katex";
import { marked } from "marked";
import markedKatex from "marked-katex-extension";

marked.use(
  markedKatex({
    throwOnError: false,
    output: "htmlAndMathml",
    nonStandard: true,
  }),
);

const cases = [
  ["行内公式", "$x^2 + y^2 = z^2$"],
  ["块级公式", "$$\n\\frac{a}{b} + \\sqrt{2}\n$$"],
  ["化学式", "反应生成水：$\\mathrm{H_2O}$"],
  ["向量/矩阵", "$\\vec{v} = \\begin{pmatrix}1 & 0 \\\\ 0 & 1\\end{pmatrix}$"],
];

let failed = 0;
for (const [name, src] of cases) {
  const html = marked.parse(src, { async: false });
  const hasKatex = html.includes('class="katex');
  const hasMathML = html.includes("<math");
  console.log(
    `${hasKatex ? "✅" : "❌"} ${name}: katex=${hasKatex} mathml=${hasMathML}`,
  );
  if (!hasKatex) failed += 1;
}

// 容错：非法公式不应抛异常（throwOnError: false）。
const bad = marked.parse("这个公式是坏的：$\\frac{", { async: false });
const tolerant = typeof bad === "string";
console.log(`${tolerant ? "✅" : "❌"} 非法公式容错（不抛异常）`);
if (!tolerant) failed += 1;

console.log(`KaTeX 版本：${katex.version}`);
if (failed) {
  console.error(`失败 ${failed} 项`);
  process.exit(1);
}
console.log("LaTeX 渲染链路自检通过");
