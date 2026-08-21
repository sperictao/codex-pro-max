# dsh 远程授权配置教程

本教程面向操作者，手把手配置 Codex Pro Max 集成卡片里的 dsh 远程访问授权。
关于链路原理、身份边界与兼容栈，见 [dsh-remote-access.md](./dsh-remote-access.md)。

## 1. 三个参数是什么

在集成卡片切换到**远程模式**后，会显示 **Remote authorization** 配置块，共有三个字段：

| 字段 | 控制什么 | 前端自动拼出的 capability | 留空时 |
| --- | --- | --- | --- |
| **Admin capability domain**（管理 capability 域名） | 远程**管理/特权**接口（settings、credentials、宿主文件等） | `<域名>/cap/dsh-admin` | 远程管理接口恒 403（仅本机可用） |
| **Use capability domain**（普通使用 capability 域名） | 普通远程 HTTP / RPC / WebSocket | `<域名>/cap/dsh` | 普通远程访问只靠身份 allowlist |
| **Extra allowed logins**（额外允许的登录名） | **谁能**远程访问（身份 allowlist） | — | 只有本机当前登录用户可访问 |

三者各自独立，**没有互斥**，可以任意组合（只配一个、配两个或全配）：

- 两个 capability 域名 = 管"**能做什么**"（权限种类）
- 额外登录名 = 管"**谁能来**"（身份名单）

> 本机当前登录用户**永远自动包含**在允许名单里，不需要也**不建议**手动加自己。

## 2. 配置前置条件

开始之前先确认：

1. **Tailscale 1.92 及以上**。转发 App Capability 需要该版本，旧版 `tailscale serve` 会报
   `unknown flag: --accept-app-caps`。
2. **一个你控制的域名**（如 `example.com`），用于 capability 命名。
   不能用 `tailscale.com` / `tailscale.io` 保留命名空间。
3. **Tailscale 在线且已登录**，MagicDNS / HTTPS Certificates 已启用（远程模式下点
   一键启动时时间轴会自动逐项检查这些前提）。

## 3. 输入规则

### capability 域名（Admin / Use 两个字段通用）

- 必须含 `.`，不以 `-` 或 `.` 开头、结尾
- 只允许 ASCII 字母数字、`-`、`.`（合法值如 `example.com`、`corp.acme.io`）
- 填的是**域名**，不带 `/cap/...` 后缀——应用会自动拼上：
  - Admin 字段 → `<域名>/cap/dsh-admin`
  - Use 字段 → `<域名>/cap/dsh`

### 额外允许的登录名

- 逗号分隔，多个账号如 `alice@example.com,bob@example.com`
- 只允许 ASCII 字母数字及 `@._+-`（Tailscale 登录名字符集）
- 自动 trim、去重，留空项会被跳过

## 4. 在应用里填写

1. 打开集成卡片，把**远程访问**开关拨到远程模式（Remote access 复选框）。
2. 出现 **Remote authorization** 块后，按你的场景填入字段。
3. 点 **Save**（按钮在 Remote authorization 标题右侧），提示"Remote authorization saved"即已落盘。

> 字段是本地草稿，只有点 Save 才会写入配置。切换访问模式或重启应用后，已保存的
> 配置会保留并在下次进入远程模式时回填显示。

## 5. 典型场景

### 场景 A：个人远程使用，无需管理

适合个人要从另一台设备（手机 / 笔记本 / 平板）远程访问 dsh，但**不需要远程管理
接口**的情况：

- 若另一台设备用的是**与运行 dsh 机器相同的 Tailscale 账号**（最常见），
  **不需要填任何账号**——Tailscale 身份是账号级的，本机当前账号已自动进入允许
  名单，且覆盖该账号在任意设备上的访问
- 只有当你要用**其它不同的 Tailscale 账号**从别处访问时，才把那个账号填进
  **Extra allowed logins**
- 两个 capability 域名留空：远程管理接口（settings / credentials）恒 403，最安全
- 不需要配置 grants（因为没用到 capability）

### 场景 B：个人远程使用，需管理（本人远程也能管理）

和场景 A 相同，但在此基础上给本人开放 admin capability，远程也能管理
settings / credentials。**需要配置**，完整链条如下：

1. **前置环境**（前提，非配置）：Tailscale 1.92+；运行 dsh 的机器已登录；
   MagicDNS + HTTPS Certificates 已启用
2. **卡片配置**（应用内，集成卡片 → 远程模式 → Remote authorization）：
   - **Admin capability domain** 填 `example.com`
   - **Use capability domain** 填 `example.com`（开放普通远程 API/WS）
   - **Extra allowed logins** 通常无需填（本机账号已包含，且身份是账号级、覆盖你
     任何设备）；仅当你用其它不同 Tailscale 账号访问时才填
   - 点 **Save**
3. **tailnet grants**（admin console，capability 生效的必须一环，见第 6 节）：
   - `src` 填你本人（账号或你所在的组）
   - `dst` 填 `autogroup:member`（个人场景最省事，不必给节点打 tag）
   - `app` 里给 `<域名>/cap/dsh` 与 `<域名>/cap/dsh-admin`

> 注意：一键启动时间轴的最后一步 verify **只验证本机 loopback 特权 API 与
> HTTPS/WSS 可达**，并不会用远程身份真正试一次 capability 授权。所以 grants 配完
> 后，最后要**从另一台设备实际访问一下**确认管理接口能打开，才算真正生效。

### 场景 C：只想给同事开远程访问，不开放管理（多用户最小配置）

只在 **Extra allowed logins** 填要允许的登录名，两个 capability 域名留空。

- 效果：列出的登录名（含本机）可远程访问 dsh Web UI
- 远程管理接口保持 403
- 不需要配置 grants（因为没用到 capability）

### 场景 D：开放远程普通访问 + 管理

- **Admin capability domain** 填 `example.com`
- **Use capability domain** 填 `example.com`
- **Extra allowed logins** 填允许的账号

### 场景 E：精细授权（不同人不同权限）

- 两个 capability 域名都填你控制的域名
- **Extra allowed logins** + **tailnet grants**（见第 6 节）里，普通用户只给
  `<域名>/cap/dsh`，管理员再额外给 `<域名>/cap/dsh-admin`

## 6. 关键一环：tailnet grants（capability 必须三处同名）

**在卡片里配了 capability 不等于生效。** 光有应用侧配置还不够，Launcher 已自动完成
前两处（注入 env + 把非空 capability 以 `--accept-app-caps=<use>,<admin>` 传给 serve），
但**剩余一环**要你在 Tailscale admin console 的 **Access controls / Policy** 里做：
给目标身份下发放行该 capability。

三处必须**同名**：

1. 你在卡片里配置的域名
2. Launcher 注入的 env / `serve --accept-app-caps`
3. tailnet grants 里的 capability 名

grants 示例（`dst` 需匹配运行 dsh 的节点，未打 tag 可用该节点身份或 `autogroup:member`；
`src` 填允许远程访问/管理的账号或组）：

```json
{
  "grants": [
    {
      "src": ["group:dsh-admins"],
      "dst": ["tag:dsh-host"],
      "app": {
        "example.com/cap/dsh": [{}],
        "example.com/cap/dsh-admin": [{}]
      }
    }
  ]
}
```

- `dst` 必须匹配运行 dsh 的节点。若没给节点打 tag，可改用该节点身份
  （形如 `user@host` 的 identity）或 `autogroup:member`。
- capability 名必须与卡片配置的域名一致：普通用户给 `<域名>/cap/dsh`，
  管理员再加 `<域名>/cap/dsh-admin`。
- 若只用了 **Use capability**（场景如普通访问需 capability），grants 里只写
  `"example.com/cap/dsh": [{}]` 即可，不必写 admin。

## 7. 端到端验证

远程模式下点**一键启动**，时间轴应依次通过 8 步：

1. Node.js 与 npm
2. 锁定版本的 dsh
3. 两个授权插件
4. Tailscale 在线与当前登录身份
5. MagicDNS / HTTPS Certificates
6. dsh 监听 `127.0.0.1:3899`
7. Tailscale Serve 直接指向 3899
8. 本地 HTTP、远程 HTTPS / WSS 和本地特权 API 验证

手动排查可用：

```bash
dsh --version
tailscale status --json
tailscale serve status
```

`tailscale serve status` 的根路由应显示 `proxy http://127.0.0.1:3899`。

## 8. 常见错误与排查

| 现象 | 原因 / 解决 |
| --- | --- |
| 保存时报"Invalid capability domain" | 域名不含 `.`，或含 `-` / `.` 之外的字符，或以 `-` / `.` 开头结尾。改用 `example.com` 这类合法域名 |
| 保存时报"Tailscale login name contains unsupported characters" | 登录名含非法字符（如空格、中文）。只允许 ASCII 字母数字及 `@._+-` |
| 启动时间轴卡在最后一步 / 远程打开提示无权限 | capability 只配了卡片没配 grants，或 grants 里 capability 名与卡片域名不一致。核对第 6 节 |
| serve 报 `unknown flag: --accept-app-caps` | Tailscale 版本过旧，需 1.92+。升级 Tailscale |
| 另一台设备打不开 `https://<hostname>.ts.net` | 多被 Shadowrocket / Clash / Surge 或系统代理抢走 tailnet 流量，访问端加 `DOMAIN-SUFFIX,ts.net,DIRECT` 直连规则（详见 dsh-remote-access.md 的排查节） |
| 远程管理接口（settings / credentials）始终 403 | Admin capability 未配置或未在 grants 下发。确认 `dsh_admin_cap_domain` 已填、grants 里给了 `<域名>/cap/dsh-admin` |

> 能力最小、最安全的起点是场景 A（个人最简使用）或场景 C（多用户最小配置）：
> 只靠本机自动允许或"额外允许登录名"， capability 是进阶选项。若不确定，先从最简
> 场景开始。
