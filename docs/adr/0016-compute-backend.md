# 验算运行时：Pyodide 跑在 GUI WebView，经 RPC 桥接

compute 内核插件负责契约（工具 schema、超时、审计、资源限制），实际执行端为 GUI WebView 内的 Pyodide（Python + SymPy/NumPy 的 WASM 构建），经既有 RPC 通道桥接；WASM 本身即沙箱（默认无文件、无网络）。GUI 离线时验算不可用，对中学生使用场景可接受。kernel 内嵌 wasmtime + Python WASM 列入后续优化，届时 compute 可完全收进 kernel、不依赖 GUI。
