// SMILES 结构式渲染：识别 ```smiles 代码块，用 smiles-drawer 绘制为 SVG。
// 约定：模型输出结构式时用 fenced code block，语言标记为 smiles，如：
//   ```smiles
//   C1=CC=CC=C1
//   ```
// 此模块在 DOMPurify 之后执行（v-html 已注入），仅接受库生成的 SVG，
// 不把用户输入当 HTML 解析，避免绕过 XSS 防线。

import SmilesDrawer from "smiles-drawer";

const DRAWER_OPTIONS = {
  width: 360,
  height: 260,
  padding: 20,
  bondLength: 60,
  compactDrawing: true,
};

let drawer = null;

function getDrawer() {
  if (!drawer) {
    drawer = new SmilesDrawer.SvgDrawer(DRAWER_OPTIONS);
  }
  return drawer;
}

function showFallback(container, smiles, err) {
  console.warn("[smiles] 结构式渲染失败:", smiles, err);
  container.classList.add("smiles-error");
  const code = document.createElement("code");
  code.textContent = smiles;
  container.append("结构式渲染失败，SMILES：");
  container.append(code);
}

/**
 * 扫描 root 内所有 ```smiles 代码块，替换为结构式 SVG。
 * @param {Element} root v-html 已注入的容器
 */
export function drawSmilesIn(root) {
  if (!root || typeof document === "undefined") return;

  root.querySelectorAll("pre > code.language-smiles").forEach((code) => {
    const pre = code.parentElement;
    const smiles = (code.textContent ?? "").trim();
    if (!smiles) return;

    const container = document.createElement("div");
    container.className = "smiles-canvas";
    pre.replaceWith(container);

    SmilesDrawer.parse(
      smiles,
      (tree) => {
        try {
          const svg = getDrawer().draw(tree, null, "light");
          container.appendChild(svg);
        } catch (err) {
          showFallback(container, smiles, err);
        }
      },
      (err) => showFallback(container, smiles, err),
    );
  });
}
