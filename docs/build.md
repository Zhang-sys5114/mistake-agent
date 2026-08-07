# 构建文档

## 前置依赖

- Rust 2024 edition toolchain（`cargo --version` 确认可用）
- Node.js + npm（前端构建；建议 Node 20+）
- **GNU Make 4.x**（`make --version` 确认可用）

### 各平台安装 Make

| 平台 | 安装方式 |
|---|---|
| Linux | 包管理器自带（apt / dnf / pacman） |
| macOS | `brew install make`（Apple 自带的 BSD make 不支持本项目用到的语法） |
| Windows | `winget install ezwinports.make`，把 `$LOCALAPPDATA\Microsoft\WinGet\Packages\ezwinports.make_*\bin` 加进系统 PATH（永久）。临时：`export PATH="$LOCALAPPDATA/Microsoft/WinGet/Packages/ezwinports.make_*/bin:$PATH"` |

桌面端依赖：Linux 需 WebKitGTK 等 Tauri 系统依赖；Windows 打包另需 NSIS（bundler 自带）/WebView2（Win11 已预装）。

## 推荐：用 Makefile 一键构建（跨平台）

仓库根的 `Makefile` 是**唯一推荐入口**。它基于文件 mtime 自动跳过未变化的阶段，**改前端不重编 Rust、改 Rust 不重跑 vite**。

```bash
make                 # 完整构建：web/dist + target/release/mistake-agent(.exe) + 安装包
make help            # 列出所有 target
make build-frontend  # 只重建 web/dist
make build-rust      # 只重编 Rust release 二进制
make bundle          # 只重打安装包
make fetch-pyodide   # 单独下载 Pyodide 离线 wheel
make clean           # 清掉 target/、web/dist/、web/node_modules/
```

### 各平台产物路径

| 平台 | 可执行文件 | 安装包 |
|---|---|---|
| Windows | `target/release/mistake-agent.exe` | `target/release/bundle/nsis/*.exe` |
| Linux   | `target/release/mistake-agent` | `target/release/bundle/{deb/*.deb,appimage/*.AppImage}` |
| macOS   | `target/release/mistake-agent` | `target/release/bundle/macos/*.dmg` |

`make bundle` 会按当前平台自动选 `BUNDLE_TARGETS`（见 Makefile 顶部 `ifeq ($(OS),Windows_NT)` 块）；若需跨平台打包，请在对应平台上跑 `make`。

## Makefile 内部做了什么

### 阶段 1 — 前端依赖

```bash
cd web && npm ci   # 用 package-lock.json 锁定的版本安装
```

触发条件：`web/package.json` 或 `web/package-lock.json` 变化。

### 阶段 2 — Pyodide 离线包

```bash
cd web && npm run fetch:pyodide   # 下载 numpy / sympy / mpmath 离线 wheel
```

**为什么必须**：`compute::verify` 的符号计算（sympy）与数值计算（numpy）依赖 Pyodide 离线 wheel。该脚本真实加载一次 Pyodide，把 wheel 缓存进 `node_modules/pyodide/`；`vite build` 会把整个目录拷进 `dist/pyodide`，运行期**本地优先、不依赖 CDN**。

- 触发条件：wheel 文件不存在
- 仅需联网一次；之后缓存在本地，重复构建不再下载
- 清过 `node_modules`（重跑 `npm ci`）后必须重跑一次

### 阶段 3 — 前端构建

```bash
cd web && npm run build   # vite build → web/dist/
```

触发条件：上一阶段产物、`web/src/`、`web/index.html`、`web/vite.config.js` 任一变化。

### 阶段 4 — Rust release 编译

```bash
cargo build --release --bins
```

触发条件：`web/dist/`、`src/**/*.rs`、`Cargo.toml`、`build.rs` 任一变化。

### 阶段 5 — Tauri 安装包

```bash
cargo tauri build --bundles nsis   # Windows
cargo tauri build --bundles deb appimage   # Linux
cargo tauri build --bundles dmg   # macOS
```

`tauri.conf.json` 中 `bundle.active=true`，bundler 才会真出安装包。

## 等效手动命令（排查 Makefile 失败时用）

如果 `make` 内部某步失败，可手动跑下面等效命令隔离问题：

```bash
# 1. 前端依赖
cd web && npm ci

# 2. Pyodide wheel（首次或清过 node_modules 后才需要）
cd web && npm run fetch:pyodide

# 3. 前端
cd web && npm run build

# 4. Rust release（前端必须先 build 完，Tauri 嵌入 web/dist）
cd ..
cargo build --release --bins

# 5. 安装包（按平台）
cargo tauri build --bundles nsis          # Windows
cargo tauri build --bundles deb appimage  # Linux
cargo tauri build --bundles dmg           # macOS
```

## 开发期常用命令

```bash
cargo test                                       # 单元测试
cargo test --test live_api -- --ignored --nocapture   # 真实 API 集成测试（需 settings.json 配置 key）
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd web && npm run check:pyodide                  # Pyodide 真实执行自检（算术/符号计算/物理/numpy）
cd web && node scripts/katex-check.mjs           # LaTeX 渲染链路自检
```

## 常见问题

- **`make: command not found`**：见「各平台安装 Make」。
- **`make help` 输出乱码或报错**：检查 `make --version ≥ 4.0`，且 shell 支持 UTF-8。
- **vite build 报"缺少 Pyodide 离线包"**：先跑 `make fetch-pyodide`。
- **check:pyodide 失败**：确认 `web/node_modules/pyodide/` 下有 `sympy-*.whl`、`numpy-*.whl`、`mpmath-*.whl`；缺失则重跑 `make fetch-pyodide`。
- **`cargo tauri build` 报 "tauri CLI not found"**：`cargo install tauri-cli --version "^2.0" --locked`，之后 `cargo tauri build` 即可。Makefile 没强制安装 tauri-cli，因为它体积大、装得慢，留给用户按需装。
- **Windows 安装包（M6 验收）**：`tauri build` 走 Tauri bundler，依赖 NSIS（bundler 自带）与 WebView2（Win11 已自带）。

## 相关文档

- README.md「构建与运行」：快速开始入口
- PROJECT.md「验收命令」：发布前验收清单
- docs/testing.md：测试策略与用例记录