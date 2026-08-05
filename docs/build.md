# 构建文档

## 前置依赖

- Rust 2024 edition toolchain（`cargo --version` 确认可用）
- Node.js + npm（前端构建；建议 Node 20+）
- 桌面端：Linux 需 WebKitGTK 等 Tauri 系统依赖；Windows 打包另需 NSIS/WebView2（见 M6，未完成）

## 构建步骤（按顺序）

### 1. 前端（web/）

```bash
cd web
npm install
npm run fetch:pyodide   # 关键步骤：预热 numpy/sympy/mpmath 离线包
npm run build           # vite 构建，产物 web/dist（含 dist/pyodide 离线运行时与 wheel）
```

**`fetch:pyodide` 为什么必须**：`compute::verify` 的符号计算（sympy）与数值计算（numpy）依赖
Pyodide 离线 wheel。该脚本真实加载一次 Pyodide，把 wheel 缓存进 `node_modules/pyodide/`；
`vite build` 会把整个目录拷进 `dist/pyodide`，运行期**本地优先、不依赖 CDN**。

- 构建期防呆：缺少 wheel 时 `vite build` 直接报错，提示先跑 `npm run fetch:pyodide`
- 该步骤需要联网一次（从 Pyodide CDN 下载 wheel）；之后缓存在本地，重复构建不再下载
- 清过 `node_modules`（重新 `npm install`）后必须重跑一次

前端自检（可选，发布前建议跑）：

```bash
npm run check:pyodide    # Pyodide 真实执行：算术/符号计算/物理/numpy/异常路径
node scripts/katex-check.mjs   # LaTeX 渲染链路自检
```

### 2. 后端（仓库根目录）

```bash
cargo build --bins       # 开发构建
cargo test               # 单元测试（80 项）
cargo clippy -- -D warnings
cargo fmt --check
```

真实 API 集成测试（需在 `~/Documents/.mistake-agent/settings.json` 配置真实 key，缺失自动跳过）：

```bash
cargo test --test live_api -- --ignored --nocapture
```

### 3. Release 二进制

```bash
# 前端必须先构建（Tauri 嵌入 web/dist）
cd web && npm install && npm run fetch:pyodide && npm run build && cd ..
cargo build --release
```

产物：`target/release/mistake-agent`（单二进制，含 Pyodide 运行时与离线 wheel，无 sidecar）。

## 常见问题

- **vite build 报"缺少 Pyodide 离线包"**：先跑 `npm run fetch:pyodide`。
- **check:pyodide 失败**：确认 `node_modules/pyodide/` 下有 `sympy-*.whl`、`numpy-*.whl`、`mpmath-*.whl`；缺失则重跑 `npm run fetch:pyodide`。
- **Windows 打包（M6 待办）**：`tauri build` 走 Tauri bundler，依赖 NSIS 与 WebView2，尚未在 Windows 真机验收。

## 相关文档

- README.md「构建与运行」：快速开始入口
- PROJECT.md「验收命令」：发布前验收清单
- docs/testing.md：测试策略与用例记录
