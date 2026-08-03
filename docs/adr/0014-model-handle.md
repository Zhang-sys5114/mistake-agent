# ModelHandle：模型作为第四个内核服务

ModelRuntime 作为内核核心组件，同时以服务句柄形式（ServiceId::Model）暴露给用户插件：插件在 requires 中声明 Model 后获得 ModelHandle，仅可调用带超时、abort 与审计的 complete(messages, tools?, signal)。凭据与 provider 适配只存在于 kernel 内部，插件永远接触不到 API key，也不自行实现 provider 调用。v2 内核服务共四个：storage、compute、memory、model。
