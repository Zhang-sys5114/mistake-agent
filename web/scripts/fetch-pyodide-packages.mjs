// 预热 Pyodide 离线包（numpy/sympy/mpmath）：真实加载一次并执行，pyodide 会把
// wheel 缓存进 node_modules/pyodide/（vite build 会整目录拷进 dist/pyodide，
// 运行期本地优先、不依赖 CDN）。构建前必须跑一次，vite 构建会校验存在性。
// 运行：npm run fetch:pyodide

import { loadPyodide } from "pyodide";
import { existsSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const pyodideDir = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../node_modules/pyodide/",
);

if (!existsSync(path.join(pyodideDir, "pyodide.asm.wasm"))) {
  console.error("✗ 未找到 Pyodide 运行时，请先 npm install");
  process.exit(1);
}

const py = await loadPyodide({ indexURL: pyodideDir });
// mpmath 需显式加载：新版 SymPy 把 mpmath 当作外部依赖（wheel 校验也要求它）。
await py.loadPackage(["numpy", "sympy", "mpmath"]);
// 冒烟：符号计算 + numpy 各跑一句，确认 wheel 可用。
py.runPython(
  "import sympy as sp, numpy as np\n" +
    "x = sp.symbols('x')\n" +
    "assert sp.solve(x**2 - 5*x + 6, x) == [2, 3]\n" +
    "assert np.sqrt(np.array([4, 9, 16])).tolist() == [2.0, 3.0, 4.0]\n",
);

const files = readdirSync(pyodideDir);
const wheels = ["mpmath", "sympy", "numpy"].map((p) =>
  files.find((f) => f.startsWith(`${p}-`) && f.endsWith(".whl")),
);
console.log(`✓ Pyodide ${py.version} 离线包就绪：${wheels.join("、")}`);
console.log("（wheel 已缓存到 node_modules/pyodide/，构建时随 dist/pyodide 打包）");
