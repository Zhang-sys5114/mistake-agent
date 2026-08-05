// Pyodide 验算执行端自检：真实加载 WASM 运行时并执行 Python（不依赖浏览器/网络）。
// 运行：npm run check:pyodide
// 与内核侧 BridgeCompute 单测、live_api compute_verify_roundtrip_real_api 互补：
// 本脚本只证明「执行端」本身可用，链路回执由 live 测试覆盖。

import { loadPyodide } from "pyodide";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const indexURL = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../node_modules/pyodide/",
);
if (!existsSync(path.join(indexURL, "sympy-1.14.0-py3-none-any.whl"))) {
  console.error("✗ 缺少 sympy 离线包，请先运行 npm run fetch:pyodide");
  process.exit(1);
}

const py = await loadPyodide({ indexURL });
await py.loadPackage(["numpy", "sympy"]);

const cases = [
  { name: "基础算术", code: "print(1 + 1)", expect: "2" },
  { name: "math 库", code: "import math\nprint(math.sqrt(144))", expect: "12.0" },
  { name: "多行与字符串", code: 'x = "错题"\nprint(f"验算:{x}")', expect: "验算:错题" },
  { name: "符号计算·解方程", code: "import sympy as sp\nx = sp.symbols('x')\nprint(sp.solve(x**2 - 5*x + 6, x))", expect: "[2, 3]" },
  { name: "符号计算·求导", code: "import sympy as sp\nx = sp.symbols('x')\nprint(sp.diff(sp.sin(x), x))", expect: "cos(x)" },
  { name: "符号计算·积分", code: "import sympy as sp\nx = sp.symbols('x')\nprint(sp.integrate(sp.exp(x), x))", expect: "exp(x)" },
  { name: "物理·单位换算", code: "from sympy.physics.units import kilometer, meter, convert_to\nprint(convert_to(1*kilometer, meter))", expect: "1000*meter" },
  { name: "物理·运动学", code: "v0 = 0.0\na = 9.8\nt = 2.0\nprint(v0 + a*t)", expect: "19.6" },
  { name: "numpy 数值", code: "import numpy as np\nprint(np.sqrt(np.array([4, 9, 16])))", expect: "[2. 3. 4.]" },
];

let failed = 0;
for (const c of cases) {
  let stdout = "";
  py.setStdout({ batched: (s) => (stdout += `${s}\n`) });
  py.setStderr({ batched: () => {} });
  await py.runPythonAsync(c.code);
  const got = stdout.trim();
  if (got !== c.expect) {
    console.error(`✗ ${c.name}：期望 ${JSON.stringify(c.expect)}，实际 ${JSON.stringify(got)}`);
    failed += 1;
  } else {
    console.log(`✓ ${c.name}：${got}`);
  }
}

// 异常路径：未捕获异常应携带 Python 错误信息（前端靠它把 stderr 回传给 kernel）。
try {
  await py.runPythonAsync("print(1 / 0)");
  console.error("✗ 除零未抛错");
  failed += 1;
} catch (e) {
  const msg = String(e?.message ?? e);
  if (msg.includes("ZeroDivisionError")) {
    console.log("✓ 异常路径：ZeroDivisionError 正确抛出");
  } else {
    console.error(`✗ 异常信息不符：${msg}`);
    failed += 1;
  }
}

console.log(`pyodide ${py.version}：${failed === 0 ? "全部通过" : `${failed} 项失败`}`);
process.exit(failed === 0 ? 0 : 1);
