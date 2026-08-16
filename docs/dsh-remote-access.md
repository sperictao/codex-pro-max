# dsh 远程访问与授权插件

Codex Pro Max 不再使用 3898 回环反代，也不会改写 `Host` / `Origin` 或伪造
`SSH_CONNECTION`。当前链路是：

```text
远程浏览器
  → https://<hostname>.ts.net  (Tailscale Serve，TLS + 身份头)
  → 127.0.0.1:3899            (dsh web，显式 loopback 绑定)
  → dsh-client-connection-authz
  → dsh-auth-tailscale
```

## 固定兼容栈

Launcher 把以下三部分当成一个不可拆分的兼容单元：

- DeepSeek Harness `0.1.0-rc.6`；
- [dsh-client-connection-authz](https://github.com/sperictao/dsh-client-connection-authz)；
- [dsh-auth-tailscale](https://github.com/sperictao/dsh-auth-tailscale)。

两个插件以固定 commit 的 Git submodule 进入源码树，构建时生成本地 `.tgz` 并作为
Tauri resource 打进安装包。运行时通过
`dsh plugin --profile web add file:<bundled-plugin>.tgz` 安装，所以不需要 Git、
GitHub 登录或运行时网络下载插件。

Connection 替代包会精确禁用内置
`@deepseek-ai/dsh-client-connection`，插入保留官方 HTTP、RPC、WebSocket 和浏览器
行为的完整替代实现，并要求注入 `connectionRequestAuthorizer`。Tailscale 插件提供
这个接口；缺插件、身份解析失败或授权配置为空时都会 fail closed。

## 身份与权限边界

一键远程访问会从 `tailscale status --json` 中把 `Self.UserID` 映射到对应的
`User[*].LoginName`，再作为精确 allowlist 传给插件。Serve 会清除客户端伪造的同名
身份头，再把真实 Tailscale 身份注入本地后端。

- dsh 只监听 `127.0.0.1:3899`，不能从 LAN 或公网绕过 Serve 直连。
- 普通远程 HTTP、RPC 与 WebSocket 必须通过 Tailscale 身份授权。
- 本机请求仍需同时满足 loopback TCP peer 与 loopback Host，才能走真实本地旁路。
- Launcher 不配置 admin App Capability，因此远程 settings、credentials、宿主文件等
  特权接口保持拒绝；本机特权接口仍可用。
- 只使用私有 Tailscale Serve，不使用 Funnel，也不把 dsh 绑定到 `0.0.0.0`。

开启或关闭自启时，Launcher 会卸载并删除旧版自己生成的 proxy plist/unit/cmd/desktop
和 `start-proxy.*`，并停止遗留的 `loopback-proxy.js` 进程；不会删除用户目录中的
`~/.dsh/loopback-proxy.js` 或其它用户文件。

## 状态检查

一键配置时间轴应依次通过：

1. Node.js 与 npm；
2. 锁定版本的 dsh；
3. 两个授权插件；
4. Tailscale 在线与当前登录身份；
5. MagicDNS / HTTPS Certificates；
6. dsh 监听 `127.0.0.1:3899`；
7. Tailscale Serve 直接指向 3899；
8. 本地 HTTP、远程 HTTPS/WSS 和本地特权 API 验证。

手动排查可使用：

```bash
dsh --version
tailscale status --json
tailscale serve status
```

`tailscale serve status` 的根路由应显示
`proxy http://127.0.0.1:3899`。设置页出现“修复 dsh 兼容栈”时，说明核心版本或
profile 中的插件 tarball 与 Launcher 锁定值不一致；点击修复会重新安装整个兼容单元。

## 访问端代理工具拦截

如果 Launcher 已验证通过，但另一台设备打不开 `https://<hostname>.ts.net`，最常见
原因是 Shadowrocket、Clash、Surge 或系统代理抢走了 tailnet 流量。访问端应让以下
规则直连：

```text
DOMAIN-SUFFIX,ts.net,DIRECT
IP-CIDR,100.64.0.0/10,DIRECT
```

Clash / Mihomo 可放在 `rules:` 最前面：

```yaml
rules:
  - DOMAIN-SUFFIX,ts.net,DIRECT
  - IP-CIDR,100.64.0.0/10,DIRECT,no-resolve
```

Surge 使用同名 `[Rule]` 项。macOS / Windows 系统代理可把 `*.ts.net` 加入 bypass。
iOS 通常只能同时运行一个 Packet Tunnel VPN；若 Shadowrocket 与 Tailscale 冲突，
断开 Shadowrocket，只保留 Tailscale。
