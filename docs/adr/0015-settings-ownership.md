# 配置与凭据：用户独占写、kernel 独占读

settings.json 是配置的唯一载体，位于数据根目录。写入只允许通过 App 设置界面（GUI 的 set_settings RPC，UserOnly）；kernel 启动时读取，API key 只在 ModelRuntime 内部使用；模型（LLM）与任何插件都没有配置访问通道，ModelHandle 也不包含配置能力。v2 采用明文存储 + 用户目录权限保护，Windows 凭据管理器（DPAPI）列入后续优化。
