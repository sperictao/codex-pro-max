# dsh 远程访问被代理工具拦截：排障指南

一键开启远程访问后，`https://<hostname>.ts.net` 打不开，最常见的原因不是链路配置，而是**访问端设备的代理工具（Shadowrocket / Clash / Surge 等）把 tailnet 流量抢走了**。

> The most common reason `https://<hostname>.ts.net` won't open after one-click setup is a proxy tool (Shadowrocket / Clash / Surge …) on the *client* device hijacking tailnet traffic — not a setup failure.

---

## 根因 | Root cause

链路本身是好的：

```
远程设备浏览器 → https://<hostname>.ts.net (Tailscale serve, 443)
              → 127.0.0.1:3898 (loopback 反代) → 127.0.0.1:3899 (dsh web)
```

但代理工具会在两处破坏它：

1. **域名被代理规则命中**：默认规则把 `*.ts.net` 当普通公网域名，转发到上游代理节点。上游节点不在你的 tailnet 里，解析不了 MagicDNS（`100.x.x.x`）→ 超时 / 连接失败。
2. **iOS 单 VPN 限制**：Shadowrocket 和 Tailscale 都是 Packet Tunnel VPN，同一时刻只能开一个。开着 Shadowrocket 时 Tailscale 隧道是断的，而 `ts.net` 的流量又被 Shadowrocket 接管 → 必然打不开。

## 自查 | Quick check

在**访问端设备**上确认：

```bash
ping <hostname>.ts.net
# 应解析到 100.x.x.x（tailnet 内网地址）。
# 若解析到公网 IP 或解析失败 → 代理/DNS 在拦截，按下面配置。
```

iOS 用户先确认：Tailscale 已连接且 Shadowrocket 已断开（状态栏 VPN 图标属于 Tailscale）。

## 各工具配置 | Per-tool fix

### Shadowrocket (iOS)

1. 配置 → 规则 → 添加规则：
   - 类型 `DOMAIN-SUFFIX`，值 `ts.net`，策略 `DIRECT`
   - 类型 `IP-CIDR`，值 `100.64.0.0/10`，策略 `DIRECT`
2. 若仍打不开：断开 Shadowrocket，只保留 Tailscale 连接（iOS 单 VPN 限制，二者不能同时全局接管）。

### Clash / ClashX / Clash Verge / Mihomo

在配置 `rules:` **最前面**加：

```yaml
rules:
  - DOMAIN-SUFFIX,ts.net,DIRECT
  - IP-CIDR,100.64.0.0/10,DIRECT,no-resolve
  # …原有规则
```

改完重载配置 / 重启内核。

### Surge

```ini
[Rule]
DOMAIN-SUFFIX,ts.net,DIRECT
IP-CIDR,100.64.0.0/10,DIRECT,no-resolve
```

### 系统代理（浏览器走系统代理设置）

macOS：系统设置 → 网络 → 代理 → “忽略这些主机与域的代理设置”加入 `*.ts.net`。
Windows：设置 → 网络 → 代理 → 例外列表加入 `*.ts.net`。

多数 GUI 代理工具（ClashX、Clash Verge 等）改完规则后会自动维护这份系统级 bypass，无需手动。

## 为什么不能用公网方案绕过

dsh 按安全设计只监听 loopback、拒绝 `--host 0.0.0.0`，敏感 API（settings/credentials）强制 loopback-only。改成公网监听或 `tailscale funnel` 会把管理面暴露到整个互联网——**不要**为了解决代理拦截而牺牲这个边界。正确做法永远是上面这种：让 tailnet 流量在客户端走 DIRECT。
