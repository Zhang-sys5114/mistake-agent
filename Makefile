# Mistake Agent 构建入口 (GNU Make 4.x)
#
# 产物（全部位于项目根 target/ 下）：
#   target/release/mistake-agent.exe         —— release 可执行文件
#   target/release/bundle/nsis/*.exe         —— NSIS 安装包（bundle.active=true 时产出）
#
# 用法：
#   make                  # 等价于 make all
#   make all              # 完整构建（前端 + Rust release + 安装包）
#   make build-frontend   # 只重建 web/dist
#   make build-rust       # 只重新编译 Rust release 二进制
#   make bundle           # 只重打安装包
#   make clean            # 清理 target/、web/dist/、web/node_modules/
#   make help             # 列出所有 target
#
# Windows 注意事项：
#   本机 GNU Make 通过 winget 装在：
#     %LOCALAPPDATA%\Microsoft\WinGet\Packages\ezwinports.make_*/bin\make.exe
#   永久解决：把该 bin/ 目录加进系统 PATH 后重启 shell。
#   临时解决（仅本会话）：export PATH=.../bin:$PATH
#
# 默认 shell：Windows 上 Make 4.4 内置 sh 即可；不需要 Git Bash。

# ---------------------------------------------------------------------------
# 工具与产物路径
# ---------------------------------------------------------------------------

# Makefile 所在目录的绝对路径 —— 保证从任意 cwd 调用都能正确工作
ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))

# Windows 上 winget 安装的 GNU Make bin 目录会自动出现在 PATH（其他平台不适用）
# 这里仅作文档注释；不要把绝对路径写死（每个用户的包目录 hash 不同）。
# 若 make 不在 PATH 里，临时解决：
#   Windows : export PATH=$LOCALAPPDATA/Microsoft/WinGet/Packages/ezwinports.make_*/bin:$PATH
#   Git Bash: export PATH="$LOCALAPPDATA/Microsoft/WinGet/Packages/ezwinports.make_*/bin:$PATH"

# 产物路径
BIN          := target/release/mistake-agent.exe
BUNDLE_DIR   := target/release/bundle
NSIS_DIR     := $(BUNDLE_DIR)/nsis
FRONTEND_DST := web/dist

# Pyodide 离线 wheel 标记（任一文件存在即视为已下载）
PYODIDE_WHEEL := web/node_modules/pyodide/numpy-2.4.3-cp314-cp314-pyemscripten_2026_0_wasm32.whl

# ---------------------------------------------------------------------------
# 默认与伪目标
# ---------------------------------------------------------------------------

.PHONY: all help build-frontend fetch-pyodide build-rust bundle clean check-tools

all: bundle ## 完整构建（默认）

help: ## 列出所有可用的 target
	@echo "可用 target："
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
	  awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

check-tools: ## 校验 node / npm / cargo 可用
	@command -v node  >/dev/null 2>&1 || { echo "[FAIL] 未检测到 node"; exit 1; }
	@command -v npm   >/dev/null 2>&1 || { echo "[FAIL] 未检测到 npm";  exit 1; }
	@command -v cargo >/dev/null 2>&1 || { echo "[FAIL] 未检测到 cargo"; exit 1; }
	@echo "[OK] node / npm / cargo 均可用"

# ---------------------------------------------------------------------------
# 阶段 1：前端依赖 + Pyodide 离线包
# ---------------------------------------------------------------------------

# 安装 npm 依赖 —— 增量：package-lock.json 变化时才重跑
web/node_modules/.package-lock.json: web/package.json web/package-lock.json
	@echo [build] npm ci 装前端依赖
	@cd web && npm ci

# 下载 Pyodide 离线 wheel —— 一次性，文件存在即跳过
fetch-pyodide: $(PYODIDE_WHEEL) ## 预热 Pyodide numpy/sympy/mpmath 离线 wheel

$(PYODIDE_WHEEL):
	@echo [build] fetch:pyodide 下载 numpy/sympy/mpmath 离线包
	@cd web && npm run fetch:pyodide

# ---------------------------------------------------------------------------
# 阶段 2：构建前端 web/dist
# ---------------------------------------------------------------------------

# web/dist 是 vite 的产物目录 —— 只要它存在就视为完成
build-frontend: $(FRONTEND_DST) ## 构建前端 (vite build)

$(FRONTEND_DST): web/node_modules/.package-lock.json $(PYODIDE_WHEEL) web/src web/index.html web/vite.config.js
	@echo "[build] vite build"
	@cd web && npm run build
	@test -d $(FRONTEND_DST) || { echo "[FAIL] vite build 未产出 $(FRONTEND_DST)"; exit 1; }

# ---------------------------------------------------------------------------
# 阶段 3：Rust release 构建
# ---------------------------------------------------------------------------

build-rust: $(BIN) ## 编译 Rust release 二进制

# 增量：src/ 或 Cargo.toml 变化才重编
$(BIN): $(FRONTEND_DST) $(wildcard src/**/*.rs src/*.rs Cargo.toml build.rs)
	@echo "[build] cargo build --release --bins"
	@cargo build --release --bins
	@test -f $(BIN) || { echo "[FAIL] cargo build 未产出 $(BIN)"; exit 1; }
	@echo "[OK] release 二进制就绪: $(BIN)"

# ---------------------------------------------------------------------------
# 阶段 4：Tauri 安装包（NSIS）
# ---------------------------------------------------------------------------

bundle: $(NSIS_DIR)/$(notdir $(BIN:.exe=.exe)) ## 打 NSIS 安装包

# NSIS 产物文件名形如 mistake-agent_0.1.0_x64-setup.exe —— 用通配符兜底
$(NSIS_DIR):
	@echo "[build] cargo tauri build（NSIS 安装包）"
	@cargo tauri build --no-bundle
	@cargo tauri build --bundles nsis
	@test -d $(NSIS_DIR) || { echo "[FAIL] tauri bundler 未产出 $(NSIS_DIR)"; exit 1; }
	@echo "[OK] 安装包就绪: $(NSIS_DIR)"

# ---------------------------------------------------------------------------
# clean
# ---------------------------------------------------------------------------

clean: ## 清理所有构建产物（target/、web/dist/、web/node_modules/）
	@echo "[clean] 删除 target/"
	@rm -rf target
	@echo "[clean] 删除 web/dist/"
	@rm -rf $(FRONTEND_DST)
	@echo "[clean] 删除 web/node_modules/"
	@rm -rf web/node_modules
	@echo "[OK] 清理完成"