# 0028 — Compute 桥接协议与命令回退

日期：2026-08-04
状态：已采纳

## 背景

M4 需要 `compute::verify` 在 GUI WebView 内的 Pyodide 执行 Python 验算。执行端与 kernel 分属两个进程（Tauri GUI ↔ sidecar），需要跨 stdio JSONL 的请求-回执协议。同时前端错题本页需要直接经 `trigger_command` 查询错题（`grading::list` 是工具而非命令）。

## 决策

1. **桥接协议**：kernel 侧 `BridgeCompute` 在收到 `ComputeService::run` 时分配自增 `compute_id`，发 `Event::ComputeRequest { id, code }` 给 GUI；GUI 执行后回 `Method::ComputeResult { compute_id, stdout, stderr, duration_ms }`；`BridgeCompute` 按 id 唤醒等待者。回执字段命名 `compute_id` 以避开 RPC 请求帧顶层 `id`（serde flatten 撞名）。
2. **超时与取消**：桥接等待带 180s 超时（`ComputeError::Timeout`）与 AbortSignal 取消；执行端未回执由超时兜底，不阻塞回合。
3. **命令回退**：`trigger_command` 找不到同名 Command 时，放行同名 UserAndModel/UserOnly 工具（`Dispatch::call_command` 回退到 `call_tool`，调用方为 User）。依据：CallerPolicy 语义下用户对两类入口点都可调，回退不破坏能力边界，且避免为每个工具重复注册命令。
4. **CSP 放宽**：Tauri CSP 增加 `script-src 'wasm-unsafe-eval'` 与 `worker-src 'self' blob:`（Pyodide 加载 WASM 必需）；仅影响本机 WebView，不引入远程源。

## 后果

- 前端协议假设：`compute_request` 事件的 id 字段回填到 `compute_result` 的 `compute_id`（web/src/lib/pyodide.js）。
- `grading::list` 保持 Tool 注册即可，前端可经 `trigger_command` 直查。
- 验算执行端依赖 Pyodide 本地打包（web/dist/pyodide），离线可用；SymPy 等 wheel 未 vendor（记录在案）。
