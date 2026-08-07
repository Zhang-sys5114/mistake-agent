// v-html-smiles 指令：renderMarkdown（KaTeX + mhchem + DOMPurify）渲染后，
// 再扫描 SMILES 代码块绘制结构式。唯一的 v-html 使用入口。

import { renderMarkdown } from "./markdown";
import { drawSmilesIn } from "./smiles";

function apply(el, value) {
  el.innerHTML = renderMarkdown(value);
  drawSmilesIn(el);
}

export const vHtmlSmiles = {
  mounted(el, binding) {
    apply(el, binding.value);
  },
  updated(el, binding) {
    if (binding.value !== binding.oldValue) {
      apply(el, binding.value);
    }
  },
};
