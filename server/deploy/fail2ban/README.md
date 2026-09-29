# fail2ban 集成

服务端把安全事件写成**固定格式**的日志行，供 fail2ban 直接匹配；同时进程内也有一套
立刻生效的失败封禁。两层互补，不要只留一层：

| 层 | 位置 | 生效速度 | 维度 | 持久性 |
|---|---|---|---|---|
| 进程内失败封禁 | `src/security/lockout.rs` | 立即 | IP **与** 账号 | 重启即清空 |
| fail2ban jail | 本目录 | 秒级 | 仅 IP | 跨重启持久 |

进程内那层负责"当场止血"（并且能封到**账号**维度——攻击者换 IP 也进不来）；
fail2ban 负责"长期记住"以及服务端被打挂时的兜底。

## 日志格式（filter 的契约）

```
AUTH_FAIL ip=<客户端IP> email=<尝试的邮箱> reason=bad_password|unknown_account|account_disabled
AUTH_BLOCK ip=<客户端IP> email=<尝试的邮箱> block_secs=900
```

改动日志格式就要同步改 `filter.d/mistake-agent.conf`，两者是一份契约。

## 安装

```bash
sudo cp filter.d/mistake-agent.conf /etc/fail2ban/filter.d/
sudo cp jail.d/mistake-agent.conf   /etc/fail2ban/jail.d/
sudo fail2ban-client reload
sudo fail2ban-client status mistake-agent
```

## 一个必须注意的前提

`ip=` 字段是不是**真实客户端 IP**，取决于 `SECURITY_TRUST_PROXY`：

- **服务端在反向代理（Caddy）后面**：设 `SECURITY_TRUST_PROXY=true`。
  服务端会取 `X-Forwarded-For` 的**最后一个**条目（我们自己的代理追加的那个），
  所以伪造头骗不到限流。
- **服务端直接对外**：保持默认 `false`。此时 `ip=` 是 TCP 对端地址，
  本机部署会恒为 `127.0.0.1`——**这种情况不要启用按 IP 的 jail**，否则会把反代或自己封掉。

判断方法：看 `/healthz` 之外的真实请求日志里 `ip=` 是否出现了多个不同地址。
