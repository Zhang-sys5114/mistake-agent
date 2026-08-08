# 构建文档

## 快速开始

```bash
# 1. 克隆仓库
git clone https://github.com/kanaD3-Chan/mistake-agent.git
cd mistake-agent

# 2. 一键构建（推荐）
make              # 完整构建：前端 + Rust release + 安装包
make help         # 列出所有可用 target
```

产物在 `target/release/` 和 `target/release/bundle/` 下。

## 前置依赖

| 工具 | 版本要求 | 用途 | 安装方式 |
|---|---|---|---|
| **Rust** | 2024 edition | 编译后端 + Tauri 应用 | [rustup.rs](https://rustup.rs/)，装完 `rustup default stable` |
| **Node.js** | 18+ (推荐 20+) | 前端构建 (Vite) + npm 依赖 | [nodejs.org](https://nodejs.org/) 或 `apt install nodejs` |
| **npm** | 随 Node.js 自带 | 前端依赖管理 | Node.js 安装包自带 |
| **GNU Make** | 4.x | 增量构建编排 | 见下表 |
| **Git** | 任意版本 | clone 仓库；Windows 上提供 sh.exe | [git-scm.com](https://git-scm.com/) |

### GNU Make 安装（按平台）

| 平台 | 命令 | 说明 |
|---|---|---|
| **Linux (Debian/Ubuntu)** | `sudo apt install make` | 包管理器自带 GNU Make 4.x |
| **Linux (Fedora/RHEL)** | `sudo dnf install make` | 同上 |
| **macOS** | `brew install make` | **必须用 Homebrew 装 GNU Make**，Apple 自带的是 BSD make 3.x，不支持本项目用到的 `$(wildcard)` / `$(info)` 等语法 |
| **Windows** | `winget install ezwinports.make` | 装完后**必须把 bin/ 加进系统 PATH**（见下方） |

#### Windows 配置 Make PATH

装完 `ezwinports.make` 后，**永久配置**（推荐）：

1. 打开"系统属性" → "高级" → "环境变量"
2. 在"系统变量"或"用户变量"的 `Path` 里新增一行：  
   `%LOCALAPPDATA%\Microsoft\WinGet\Packages\ezwinports.make_Microsoft.Winget.Source_8wekyb3d8bbwe\bin`
3. 重启 PowerShell / cmd / Git Bash

**临时配置**（单次会话）：

```powershell
# PowerShell
$env:PATH="$env:LOCALAPPDATA\Microsoft\WinGet\Packages\ezwinports.make_Microsoft.Winget.Source_8wekyb3d8bbwe\bin;$env:PATH"
make help
```

```bash
# Git Bash
export PATH="$LOCALAPPDATA/Microsoft/WinGet/Packages/ezwinports.make_Microsoft.Winget.Source_8wekyb3d8bbwe/bin:$PATH"
make help
```

验证安装：`make --version` 应输出 `GNU Make 4.4.1`。

### Windows 额外依赖（打包时）

- **Git for Windows**（Makefile 内部用 Git Bash 的 `sh.exe` 执行 recipe；已装 Git 即可）
- **NSIS**（Tauri bundler 自带，无需手动安装）
- **WebView2**（Win10 1809+ / Win11 已预装；Win10 早期版本需手动装 [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)）

### Linux 额外依赖（桌面环境）

Tauri 依赖 WebKitGTK：

```bash
# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev

# Fedora
sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel

# Arch
sudo pacman -S webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg
```

## 构建步骤（Makefile 一键）

```bash
make               # 完整构建（默认 = make all）
make help          # 列出所有 target
make build-frontend   # 只重建前端
make build-rust       # 只重编 Rust release 二进制
make bundle           # 只重打安装包（需先 build-rust）
make fetch-pyodide    # 单独下载 Pyodide 离线 wheel
make clean            # 清掉 target/、web/dist/、web/node_modules/
make check-tools      # 校验 node / npm / cargo 是否可用
```

### Makefile 内部做了什么

#### 阶段 1 — 前端依赖

```bash
cd web && npm ci   # 用 package-lock.json 锁定版本安装
```

触发条件：`web/package.json` 或 `web/package-lock.json` 变化。

#### 阶段 2 — Pyodide 离线包

```bash
cd web && npm run fetch:pyodide   # 下载 numpy / sympy / mpmath wheel
```

**为什么必须：** `compute::verify` 的符号计算（sympy）与数值计算（numpy）依赖 Pyodide 离线 wheel。该脚本真实加载一次 Pyodide，把 wheel 缓存进 `node_modules/pyodide/`；`vite build` 会把整个目录拷进 `dist/pyodide/`，运行期**本地优先、不依赖 CDN**。

- 触发条件：wheel 文件不存在（`web/node_modules/pyodide/numpy-*.whl` 等）
- 仅需联网一次；之后缓存在本地，重复构建不再下载
- 清过 `node_modules`（重跑 `npm ci`）后必须重跑一次

#### 阶段 3 — 前端构建

```bash
cd web && npm run build   # vite build → web/dist/
```

触发条件：上一阶段产物、`web/src/`、`web/index.html`、`web/vite.config.js` 任一变化。

#### 阶段 4 — Rust release 编译

```bash
cargo build --release --bins
```

触发条件：`web/dist/`、`src/**/*.rs`、`Cargo.toml`、`build.rs` 任一变化。

Tauri 的 `build.rs` 会把 `web/dist/` 嵌入二进制，所以前端必须先构建完。

#### 阶段 5 — Tauri 安装包

```bash
cargo tauri build --bundles nsis          # Windows
cargo tauri build --bundles deb appimage  # Linux
cargo tauri build --bundles dmg           # macOS
```

`tauri.conf.json` 中 `bundle.active=true`，bundler 才会真出安装包。Makefile 按当前平台自动选 `BUNDLE_TARGETS`。

### 构建产物

| 平台 | 可执行文件 | 安装包 |
|---|---|---|
| **Windows** | `target/release/mistake-agent.exe` | `target/release/bundle/nsis/错题 Agent_0.1.0_x64-setup.exe` |
| **Linux** | `target/release/mistake-agent` | `target/release/bundle/deb/*.deb`<br>`target/release/bundle/appimage/*.AppImage` |
| **macOS** | `target/release/mistake-agent` | `target/release/bundle/macos/*.dmg` |

可执行文件是**单二进制**（包含 web 资源 + Pyodide 运行时 + 离线 wheel），无 sidecar 依赖。

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
cargo tauri build --bundles nsis                # Windows
cargo tauri build --bundles deb appimage        # Linux
cargo tauri build --bundles dmg                 # macOS
```

## 开发期常用命令

```bash
# Rust 单元测试
cargo test

# 真实 API 集成测试（需在 ~/Documents/.mistake-agent/settings.json 配置 key）
cargo test --test live_api -- --ignored --nocapture

# 代码质量检查
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# 前端自检
cd web && npm run check:pyodide              # Pyodide 真实执行（算术/符号计算/物理/numpy）
cd web && node scripts/katex-check.mjs       # LaTeX 渲染链路自检

# 只重编 Rust（前端未变）
make build-rust

# 只重建前端
make build-frontend
```

## 常见问题

### `make: command not found`

**原因：** GNU Make 未安装或未在 PATH 里。

**解决：**
- Linux/macOS：`sudo apt install make` / `brew install make`
- Windows：`winget install ezwinports.make`，装完把 bin/ 加进系统 PATH（见"前置依赖" → "Windows 配置 Make PATH"）

### `make help` 输出乱码或报错

**原因：** Make 版本过低（< 4.0）或 shell 不支持 UTF-8。

**解决：**
- 检查 `make --version`，确认 ≥ 4.0
- Windows 确认 PowerShell 代码页是 UTF-8：`chcp 65001`

### vite build 报"缺少 Pyodide 离线包"

**原因：** 没跑过 `npm run fetch:pyodide`，或清过 `node_modules` 后没重跑。

**解决：** `make fetch-pyodide` 或 `cd web && npm run fetch:pyodide`

### `check:pyodide` 失败

**原因：** `web/node_modules/pyodide/` 下缺少 `sympy-*.whl` / `numpy-*.whl` / `mpmath-*.whl`。

**解决：** 重跑 `make fetch-pyodide`

### `cargo tauri build` 报 "tauri CLI not found"

**原因：** 没装 `tauri-cli`。

**解决：** `cargo install tauri-cli --version "^2.0" --locked`  
（Makefile 没强制安装 tauri-cli，因为它体积大、装得慢，留给用户按需装）

### Windows 打包报错 "NSIS not found"

**原因：** Tauri bundler 自带 NSIS，但可能因网络问题下载失败。

**解决：** 检查网络，或手动装 [NSIS](https://nsis.sourceforge.io/)（Tauri 会自动探测系统安装的 NSIS）

### Linux 构建报 "webkit2gtk not found"

**原因：** 缺 Tauri 依赖的桌面环境库。

**解决：** 见"前置依赖" → "Linux 额外依赖"，装 WebKitGTK 等

## 跨平台说明

Makefile 已完全适配 Windows / Linux / macOS：

- **help / check-tools** 用 Make 内置 `$(info)` 函数，不依赖 shell
- **build recipe** 只调 npm / cargo，它们自己会报错，Make 自动中断
- **clean** 用 Make 的 `$(RM)` 变量（自动映射为 `rm -rf`）
- **Windows SHELL 探测** 自动找 Git Bash 的 `sh.exe`（`C:/Program Files/Git/usr/bin/sh.exe`），其他平台用系统默认 `/bin/sh`

理论上任意 POSIX shell（bash / sh / dash / zsh）都能跑通。实测：
- ✅ Windows PowerShell
- ✅ Windows Git Bash
- ✅ Windows cmd (通过 Git Bash sh.exe)
- 理论 ✅ Linux / macOS（所有 recipe 只用标准工具）

## 相关文档

- [README.md](../README.md)「构建与运行」：快速开始入口
- [PROJECT.md](../PROJECT.md)「验收命令」：发布前验收清单
- [docs/testing.md](testing.md)：测试策略与用例记录