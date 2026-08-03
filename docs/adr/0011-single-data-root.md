# 单一数据根目录，无项目概念

本 Agent 不引入"项目"概念，所有数据与配置统一存放于 ~/Documents/.mistake-agent（Windows 上 ~ 即 %USERPROFILE%）：指令文件、会话、记忆、错题本、审计、设置。指令加载因此只有全局单文件 AGENTS.md，无分层合并。安装器负责创建目录与首次初始化。已注意到 Windows 的 Documents 可能被 OneDrive 重定向、导致本地数据随云同步，v2 接受该行为，必要时后续提供数据目录可配置。
