//! DeepSeek Harness (dsh) 远程访问：Tailscale 一键配置。
//!
//! 架构参照教程《用 Tailscale 远程访问 DeepSeek Harness (dsh) 完整教程》：
//! ```text
//! 远程设备 (Tailscale 内网)
//!   ▼
//! https://<hostname>.ts.net   (tailscale serve, HTTPS 443)
//!   ▼
//! 127.0.0.1:3898              (loopback 反代: Host→127.0.0.1, 删 Origin)
//!   ▼
//! 127.0.0.1:3899              (dsh web)
//! ```
//!
//! 关键事实（dsh 安全设计，教程实测）：
//! - dsh 拒绝 `--host 0.0.0.0`，只监听 loopback → 必须走 Tailscale 内网隧道
//! - 目录选择器在远程下要设 `SSH_CONNECTION` 强制 browse 模式（否则 502）
//! - 普通 API（listDirectory 等）需 `--trusted-host <hostname>.ts.net`（否则 403）
//! - 敏感 API（settings/credentials 等）强制 loopback-only，需 loopback 反代
//!   改写 Host 并删除 Origin（否则 403）
//! - 反代必须支持 WebSocket upgrade（/api/events.host），否则对话 Load failed
//!
//! 跨平台：安装/检测走 CLI（npm / tailscale / node），
//! 开机自启走 launchd(macOS) / schtasks(Windows) / systemd --user(Linux)。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::Emitter;

use crate::config;
use crate::i18n::{tr, trf};

/// dsh web 端口（与教程一致）
const WEB_PORT: u16 = 3899;
/// loopback 反代端口（与教程一致）
const PROXY_PORT: u16 = 3898;
/// 反代脚本文件名（位于 ~/.dsh/）
const PROXY_SCRIPT: &str = "loopback-proxy.js";
/// 强制 browse 目录选择器的环境变量（教程第五步）
const SSH_CONNECTION_ENV: &str = "127.0.0.1 60000 127.0.0.1 22";
/// 自启标签前缀（本应用自有的 launchd/schtasks/systemd 命名）
const AUTOSTART_PREFIX: &str = "com.codexpromax.dsh";

// ============ 数据结构 ============

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DshStatus {
    pub node_available: bool,
    pub dsh_installed: bool,
    pub dsh_version: Option<String>,
    pub dsh_running: bool,
    pub tailscale_installed: bool,
    pub tailscale_online: bool,
    pub hostname: Option<String>,
    pub url: Option<String>,
    pub magic_dns_enabled: bool,
    pub serve_configured: bool,
    pub proxy_running: bool,
    pub proxy_configured: bool,
    pub autostart_enabled: bool,
    /// 检测过程中的错误信息（无则 None）
    pub error: Option<String>,
}

/// 时间轴节点事件（dsh-step），由 dsh_setup 逐步发出
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    pub index: usize,
    pub id: String,
    /// running | done | failed | skipped
    pub state: String,
    pub detail: Option<String>,
    /// 问题描述（失败节点展示）
    pub problem: Option<String>,
    /// 解决方案（失败节点展示）
    pub solution: Option<String>,
}

// ============ 跨平台 CLI 辅助 ============

/// Windows 上 npm/全局包是 .cmd 批处理，CreateProcess 不能直接执行，
/// 必须经 cmd /c 由 cmd 做 PATHEXT 解析，且不弹控制台窗口（同 fastctx.rs）
fn cli_command(program: &str, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut cmd = Command::new("cmd");
        cmd.arg("/c").arg(program).args(args);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }
}

/// GUI 应用（Finder 启动）PATH 很薄，探测常见 CLI 位置补进 PATH
fn probe_path() -> String {
    let home = config::home_dir().unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        parts.push("/opt/homebrew/bin".to_string());
        parts.push("/usr/local/bin".to_string());
        if let Some(h) = home.to_str() {
            parts.push(format!("{}/.npm-global/bin", h));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            parts.push(format!("{}\\npm", local));
        }
        if let Ok(pf) = std::env::var("ProgramFiles") {
            parts.push(format!("{}\\nodejs", pf));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(h) = home.to_str() {
            parts.push(format!("{}/.local/bin", h));
            parts.push(format!("{}/.npm-global/bin", h));
        }
        parts.push("/usr/local/bin".to_string());
    }
    parts.push("/usr/bin".to_string());
    parts.push("/bin".to_string());
    #[cfg(windows)]
    if let Ok(sys) = std::env::var("SystemRoot") {
        parts.push(sys);
    }
    if let Ok(cur) = std::env::var("PATH") {
        parts.push(cur);
    }
    parts.join(if cfg!(windows) { ";" } else { ":" })
}

/// 跑命令并捕获 (stdout, stderr, 成功)。命令经 probe PATH 执行
fn run_capture(program: &str, args: &[&str]) -> Result<(String, String, bool), String> {
    let output = cli_command(program, args)
        .env("PATH", probe_path())
        .output()
        .map_err(|e| trf("Cannot execute {program}: {error}", &[
            ("program", program.to_string()),
            ("error", e.to_string()),
        ]))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok((stdout, stderr, output.status.success()))
}

/// 在 probe PATH 中定位可执行文件（unix: command -v；windows: where）
fn which(program: &str) -> Option<String> {
    #[cfg(unix)]
    {
        let quoted = program.replace('\'', "'\\''");
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("command -v '{}'", quoted))
            .env("PATH", probe_path())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
    #[cfg(windows)]
    {
        let out = Command::new("cmd")
            .args(["/c", "where", program])
            .env("PATH", probe_path())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let first = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().to_string();
        if first.is_empty() { None } else { Some(first) }
    }
}

/// 端口是否已有进程监听（dsh 运行状态的权威判断，跨平台）
fn port_listening(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(400),
    )
    .is_ok()
}

/// 轮询等待端口就绪；超时返回最终状态
fn wait_port(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if port_listening(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    port_listening(port)
}

/// 后台启动进程并立即返回（孤儿进程继续运行；日志重定向到文件）。
/// 说明：dsh web / 反代是常驻服务，不能随启动器退出而被杀，
/// 这里不持有 Child 句柄，与 ProcessManager 的随窗停服务语义刻意不同
fn spawn_detached(program: &str, args: &[&str], envs: &[(&str, &str)], log: &Path) -> Result<(), String> {
    let dir = log.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)
        .map_err(|e| trf("Failed to create directory: {error}", &[("error", e.to_string())]))?;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| trf("Cannot open log file: {error}", &[("error", e.to_string())]))?;
    let mut cmd = cli_command(program, args);
    cmd.env("PATH", probe_path())
        .stdout(std::process::Stdio::from(file.try_clone().map_err(|e| e.to_string())?))
        .stderr(std::process::Stdio::from(file));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.spawn()
        .map_err(|e| trf("Cannot start process: {error}", &[("error", e.to_string())]))?;
    Ok(())
}

/// 按命令行特征杀进程（unix: pkill；windows: powershell）
fn kill_by_pattern(pattern: &str) {
    #[cfg(unix)]
    {
        let _ = Command::new("pkill").arg("-f").arg(pattern).output();
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{p}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            p = pattern.replace('\'', "''")
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output();
    }
}

// ============ 组件定位 ============

fn dsh_dir() -> Result<PathBuf, String> {
    Ok(config::home_dir()?.join(".dsh"))
}

/// 定位 node 可执行（绝对路径，供自启脚本嵌入）
fn resolve_node_bin() -> Result<String, String> {
    which("node").ok_or_else(|| tr("Node.js is not available; please install Node.js 18+ and restart this app"))
}

/// 定位 npm（probe PATH 内；失败返回裸 "npm" 让错误自然暴露）
fn npm_bin() -> String {
    which("npm").unwrap_or_else(|| "npm".to_string())
}

/// dsh --version 输出；未安装返回 None
fn dsh_version() -> Option<String> {
    let bin = which("dsh")?;
    let (out, _, ok) = run_capture(&bin, &["--version"]).ok()?;
    if !ok {
        return None;
    }
    let v = out.trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

/// 定位 dsh 可执行：先 probe PATH，再经 `npm prefix -g` 推 npm 全局 bin
fn resolve_dsh_bin() -> Result<PathBuf, String> {
    if let Some(p) = which("dsh") {
        return Ok(PathBuf::from(p));
    }
    if let Ok((out, _, ok)) = run_capture(&npm_bin(), &["prefix", "-g"]) {
        if ok {
            let prefix = PathBuf::from(out.trim());
            #[cfg(windows)]
            let candidates = [
                prefix.join("dsh.cmd"),
                prefix.join("dsh.ps1"),
                prefix.join("dsh"),
            ];
            #[cfg(not(windows))]
            let candidates = [prefix.join("bin").join("dsh")];
            for c in candidates {
                if c.exists() {
                    return Ok(c);
                }
            }
        }
    }
    Err(tr("Cannot locate the dsh CLI; install it with npm install -g @deepseek-ai/dsh"))
}

/// 定位 tailscale CLI（Windows 默认装在 Program Files，不在 PATH）
fn tailscale_path() -> Option<String> {
    if let Some(p) = which("tailscale") {
        return Some(p);
    }
    #[cfg(windows)]
    for c in [
        "C:\\Program Files\\Tailscale\\tailscale.exe",
        "C:\\Program Files (x86)\\Tailscale\\tailscale.exe",
    ] {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    None
}

/// 解析 MagicDNS 状态与后缀（tailscale dns status）
fn magic_dns_info(ts: &str) -> (bool, Option<String>) {
    let Ok((out, _, _)) = run_capture(ts, &["dns", "status"]) else {
        return (false, None);
    };
    let enabled = out
        .lines()
        .any(|l| l.to_lowercase().contains("magicdns: enabled"));
    let suffix = out.lines().find_map(|l| {
        l.find("suffix = ")
            .map(|i| l[i + "suffix = ".len()..].trim_end_matches(')').trim().to_string())
    });
    (enabled, suffix)
}

/// 解析 tailnet 主机名与 HTTPS URL：
/// 1) tailscale serve status 里的 https://<host>.ts.net
/// 2) tailscale status 首行设备名 + MagicDNS 后缀
fn resolve_host_and_url() -> (Option<String>, Option<String>) {
    let Some(ts) = tailscale_path() else {
        return (None, None);
    };
    if let Ok((out, _, ok)) = run_capture(&ts, &["serve", "status"]) {
        if ok {
            for line in out.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("https://") {
                    let host = rest.split([' ', '/']).next().unwrap_or("");
                    if !host.is_empty() {
                        return (
                            Some(host.split('.').next().unwrap_or(host).to_string()),
                            Some(format!("https://{}", host)),
                        );
                    }
                }
            }
        }
    }
    if let Ok((out, _, ok)) = run_capture(&ts, &["status"]) {
        if ok {
            if let Some(first) = out.lines().next() {
                let host = first.split_whitespace().nth(1).unwrap_or("").to_string();
                if !host.is_empty() {
                    let (_, suffix) = magic_dns_info(&ts);
                    if let Some(sfx) = suffix {
                        return (Some(host.clone()), Some(format!("https://{}.{}", host, sfx)));
                    }
                    return (Some(host), None);
                }
            }
        }
    }
    (None, None)
}

/// 解析 serve 是否已指向本地端口
fn serve_configured(ts: &str) -> bool {
    match run_capture(ts, &["serve", "status"]) {
        Ok((out, _, ok)) => ok && out.contains("proxy") && out.contains(&PROXY_PORT.to_string()),
        Err(_) => false,
    }
}

/// 解析 tailnet 完全限定主机名（--trusted-host 用）：
/// 设备名 + MagicDNS 后缀，如 etmacminim4.taildde4.ts.net
fn resolve_fqdn() -> Option<String> {
    let (host, _) = resolve_host_and_url();
    let host = host?;
    if host.contains('.') {
        return Some(host);
    }
    let suffix = tailscale_path().and_then(|ts| magic_dns_info(&ts).1);
    match suffix {
        Some(s) => Some(format!("{}.{}", host, s)),
        None => Some(format!("{}.ts.net", host)),
    }
}

/// tailscale 是否在线（tailscale status 成功即在线）
fn tailscale_online(ts: &str) -> bool {
    matches!(run_capture(ts, &["status"]), Ok((_, _, true)))
}

/// 极简 HTTP GET（本地验证用；不引网络库）
fn http_get(port: u16, host_header: &str, path: &str) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    let mut s = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_secs(3),
    )
    .ok()?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host_header
    );
    s.write_all(req.as_bytes()).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    String::from_utf8_lossy(&buf)
        .lines()
        .next()
        .map(|l| l.to_string())
}

/// 状态码首行是否成功（200/3xx 均视为可达；dsh 根路径可能 302 到登录页）
fn http_ok(line: Option<&str>) -> bool {
    match line {
        Some(l) => {
            let status = l.split_whitespace().nth(1).unwrap_or("");
            status.starts_with('2') || status.starts_with('3')
        }
        None => false,
    }
}

// ============ 反代脚本（教程第六步，含 WebSocket upgrade） ============

const PROXY_JS: &str = r#"// dsh loopback 反代：让远程请求以 loopback 身份进入 dsh
// 1. PRIVILEGED_METHODS（settings/credentials 等）强制 loopback-only：
//    改写 Host 为 127.0.0.1、删除 Origin，使 dsh 判定为本地同源请求
// 2. 支持 WebSocket upgrade（/api/events.host 实时通道），否则对话报 Load failed
const http = require('http');

const UPSTREAM = { host: '127.0.0.1', port: 3899 };
const LISTEN_PORT = 3898;

// 构造转发头：改写 Host 为 loopback，删除浏览器标记头
function makeHeaders(req) {
  const headers = { ...req.headers };
  headers.host = '127.0.0.1';      // 改写 Host 为 loopback
  delete headers.origin;            // 删除 Origin（敏感 API 要求无 Origin）
  delete headers['sec-fetch-site'];
  delete headers['sec-fetch-mode'];
  delete headers['sec-fetch-dest'];
  delete headers['sec-ch-ua'];
  delete headers['sec-ch-ua-mobile'];
  delete headers['sec-ch-ua-platform'];
  return headers;
}

const server = http.createServer((req, res) => {
  const upstreamReq = http.request({
    host: UPSTREAM.host,
    port: UPSTREAM.port,
    path: req.url,
    method: req.method,
    headers: makeHeaders(req),
  }, (upstreamRes) => {
    res.writeHead(upstreamRes.statusCode, upstreamRes.headers);
    upstreamRes.pipe(res);
  });
  upstreamReq.on('error', (err) => {
    res.writeHead(502, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ error: 'upstream error', message: err.message }));
  });
  req.pipe(upstreamReq);
});

// WebSocket upgrade 转发（/api/events.host 等实时通道）
server.on('upgrade', (req, socket, head) => {
  const upstreamReq = http.request({
    host: UPSTREAM.host,
    port: UPSTREAM.port,
    path: req.url,
    method: req.method,
    headers: makeHeaders(req),
  });
  upstreamReq.on('upgrade', (upstreamRes, upstreamSocket, upstreamHead) => {
    socket.write(
      'HTTP/1.1 101 Switching Protocols\r\n' +
      `Upgrade: ${upstreamRes.headers.upgrade || 'websocket'}\r\n` +
      `Connection: ${upstreamRes.headers.connection || 'Upgrade'}\r\n` +
      `Sec-WebSocket-Accept: ${upstreamRes.headers['sec-websocket-accept'] || ''}\r\n\r\n`
    );
    if (upstreamHead && upstreamHead.length) socket.write(upstreamHead);
    upstreamSocket.pipe(socket);
    socket.pipe(upstreamSocket);
  });
  upstreamReq.on('error', (err) => {
    socket.destroy();
  });
  upstreamReq.end(head || undefined);
});

server.listen(LISTEN_PORT, '127.0.0.1', () => {
  console.log(`dsh-loopback-proxy listening on 127.0.0.1:${LISTEN_PORT} -> ${UPSTREAM.host}:${UPSTREAM.port} (HTTP + WebSocket)`);
});
"#;

/// 确保 ~/.dsh/loopback-proxy.js 存在（缺失则写入教程版脚本）
fn ensure_proxy_script() -> Result<PathBuf, String> {
    let dir = dsh_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| trf("Failed to create directory: {error}", &[("error", e.to_string())]))?;
    let path = dir.join(PROXY_SCRIPT);
    if !path.exists() {
        fs::write(&path, PROXY_JS)
            .map_err(|e| trf("Failed to write {path}: {error}", &[
                ("path", path.display().to_string()),
                ("error", e.to_string()),
            ]))?;
    }
    Ok(path)
}

// ============ 检测 ============

#[tauri::command]
pub async fn dsh_detect() -> Result<DshStatus, String> {
    let (hostname, url) = resolve_host_and_url();
    let ts = tailscale_path();
    let (magic, _) = match &ts {
        Some(p) => magic_dns_info(p),
        None => (false, None),
    };
    Ok(DshStatus {
        node_available: which("node").is_some(),
        dsh_installed: dsh_version().is_some(),
        dsh_version: dsh_version(),
        dsh_running: port_listening(WEB_PORT),
        tailscale_installed: ts.is_some(),
        tailscale_online: ts.as_deref().map(tailscale_online).unwrap_or(false),
        hostname,
        url,
        magic_dns_enabled: magic,
        serve_configured: ts.as_deref().map(serve_configured).unwrap_or(false),
        proxy_running: port_listening(PROXY_PORT),
        proxy_configured: dsh_dir()
            .map(|d| d.join(PROXY_SCRIPT).exists())
            .unwrap_or(false),
        autostart_enabled: autostart_enabled(),
        error: None,
    })
}

// ============ 一键启动（时间轴事件流） ============

fn emit_step(
    app: &tauri::AppHandle,
    index: usize,
    id: &str,
    state: &str,
    detail: Option<String>,
    problem: Option<String>,
    solution: Option<String>,
) {
    let _ = app.emit(
        "dsh-step",
        StepEvent {
            index,
            id: id.to_string(),
            state: state.to_string(),
            detail,
            problem,
            solution,
        },
    );
}

struct StepCtx<'a> {
    app: &'a tauri::AppHandle,
    index: usize,
    id: &'static str,
}

impl StepCtx<'_> {
    fn running(&self, detail: &str) {
        emit_step(self.app, self.index, self.id, "running", Some(detail.to_string()), None, None);
    }
    fn done(&self, detail: &str) {
        emit_step(self.app, self.index, self.id, "done", Some(detail.to_string()), None, None);
    }
    /// 失败：发出 failed 节点 + 把后续步骤标记 skipped，再返回 Err（时间轴即展示面）
    fn fail(&self, problem: &str, solution: &str, remaining: &[(&'static str, usize)]) -> Result<(), String> {
        emit_step(self.app, self.index, self.id, "failed", None, Some(problem.to_string()), Some(solution.to_string()));
        for (id, idx) in remaining {
            emit_step(self.app, *idx, id, "skipped", None, None, None);
        }
        Err(problem.to_string())
    }
}

#[tauri::command]
pub async fn dsh_setup(app: tauri::AppHandle) -> Result<(), String> {
    let steps: [&'static str; 8] = [
        "node", "install", "start", "tailscale", "magicdns", "proxy", "serve", "verify",
    ];
    let remaining_after = |cur: usize| -> Vec<(&'static str, usize)> {
        steps
            .iter()
            .enumerate()
            .filter(|(i, _)| *i > cur)
            .map(|(i, s)| (*s, i))
            .collect()
    };

    // —— 0. node：检测 Node.js 与 npm ——
    {
        let ctx = StepCtx { app: &app, index: 0, id: steps[0] };
        ctx.running(&tr("Checking Node.js & npm…"));
        let node = match resolve_node_bin() {
            Ok(n) => n,
            Err(e) => {
                return ctx.fail(
                    &e,
                    &tr("Install Node.js 18+ from https://nodejs.org, then restart this app and retry"),
                    &remaining_after(0),
                )
            }
        };
        let (npm_v, _, npm_ok) = run_capture(&npm_bin(), &["--version"]).unwrap_or_default();
        let node_v = cli_command(&node, &["--version"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let mut detail = format!("Node {}", node_v.trim());
        if npm_ok {
            detail.push_str(&format!(" · npm {}", npm_v.trim()));
        }
        ctx.done(&detail);
    }

    // —— 1. install：安装 dsh CLI ——
    {
        let ctx = StepCtx { app: &app, index: 1, id: steps[1] };
        match dsh_version() {
            Some(v) => ctx.done(&trf("Already installed: {version}", &[("version", v)])),
            None => {
                ctx.running(&tr("Installing dsh via npm install -g @deepseek-ai/dsh…"));
                match run_capture(&npm_bin(), &["install", "-g", "@deepseek-ai/dsh"]) {
                    Ok((_, _, true)) => match dsh_version() {
                        Some(v) => ctx.done(&trf("Installed {version}", &[("version", v)])),
                        None => {
                            return ctx.fail(
                                &tr("dsh installed but cannot be located in PATH"),
                                &tr("Restart this app so it can pick up the new PATH, or install Node.js and run npm install -g @deepseek-ai/dsh manually"),
                                &remaining_after(1),
                            )
                        }
                    },
                    Ok((_, err, _)) => {
                        let e = if err.is_empty() { "npm install failed".to_string() } else { err };
                        return ctx.fail(
                            &trf("Install failed: {error}", &[("error", e)]),
                            &tr("Check your network/proxy settings, or run npm install -g @deepseek-ai/dsh in a terminal and retry"),
                            &remaining_after(1),
                        )
                    }
                    Err(e) => {
                        return ctx.fail(
                            &e,
                            &tr("Make sure Node.js is installed and available in PATH, then retry"),
                            &remaining_after(1),
                        )
                    }
                }
            }
        }
    }

    // —— 2. start：启动 dsh web ——
    {
        let ctx = StepCtx { app: &app, index: 2, id: steps[2] };
        if port_listening(WEB_PORT) {
            ctx.done(&tr("Already running on 127.0.0.1:3899"));
        } else {
            let dsh_bin = match resolve_dsh_bin() {
                Ok(b) => b,
                Err(e) => {
                    return ctx.fail(&e, &tr("Install dsh first, then retry"), &remaining_after(2))
                }
            };
            let mut args: Vec<String> = vec![
                "--profile".into(),
                "web".into(),
                "--port".into(),
                WEB_PORT.to_string(),
            ];
            // --trusted-host 放行非敏感 API（目录浏览等）；未知主机名时省略（反代链路不依赖它）
            if let Some(fqdn) = resolve_fqdn() {
                args.push("--trusted-host".into());
                args.push(fqdn);
            }
            let log = dsh_dir().map(|d| d.join("dsh-web.log")).unwrap_or_else(|_| PathBuf::from("dsh-web.log"));
            ctx.running(&tr("Starting dsh web (127.0.0.1:3899)…"));
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            if let Err(e) = spawn_detached(
                &dsh_bin.display().to_string(),
                &arg_refs,
                &[("SSH_CONNECTION", SSH_CONNECTION_ENV)],
                &log,
            ) {
                return ctx.fail(
                    &e,
                    &tr("Port 3899 may be occupied; stop the process using it and retry"),
                    &remaining_after(2),
                );
            }
            if !wait_port(WEB_PORT, Duration::from_secs(20)) {
                return ctx.fail(
                    &tr("dsh web did not start within 20s"),
                    &trf("Check the log at ~/.dsh/dsh-web.log; port 3899 may be occupied or the dsh CLI may need a newer Node.js", &[]),
                    &remaining_after(2),
                );
            }
            ctx.done(&tr("dsh web is running on 127.0.0.1:3899"));
        }
    }

    // —— 3. tailscale：检测 Tailscale 与登录状态 ——
    {
        let ctx = StepCtx { app: &app, index: 3, id: steps[3] };
        ctx.running(&tr("Checking Tailscale…"));
        let ts = match tailscale_path() {
            Some(t) => t,
            None => {
                return ctx.fail(
                    &tr("Tailscale is not installed"),
                    &tr("Install Tailscale and sign in: macOS `brew install --cask tailscale`, Windows `winget install Tailscale.Tailscale`, Linux `curl -fsSL https://tailscale.com/install.sh | sh`, then run `tailscale up`"),
                    &remaining_after(3),
                )
            }
        };
        if !tailscale_online(&ts) {
            return ctx.fail(
                &tr("Tailscale is not connected"),
                &tr("Run `tailscale up` and sign in with the same Tailscale account you want to access from"),
                &remaining_after(3),
            );
        }
        let (_, url) = resolve_host_and_url();
        let detail = match &url {
            Some(u) => trf("Online · {url}", &[("url", u.clone())]),
            None => tr("Online"),
        };
        ctx.done(&detail);
    }

    // —— 4. magicdns：MagicDNS（HTTPS 证书依赖） ——
    {
        let ctx = StepCtx { app: &app, index: 4, id: steps[4] };
        let Some(ts) = tailscale_path() else {
            return ctx.fail(
                &tr("Tailscale is not installed"),
                &tr("Install Tailscale first, then retry"),
                &remaining_after(4),
            );
        };
        ctx.running(&tr("Checking MagicDNS…"));
        let (enabled, _) = magic_dns_info(&ts);
        if !enabled {
            return ctx.fail(
                &tr("MagicDNS is not enabled"),
                &tr("Open https://login.tailscale.com/admin/dns and enable MagicDNS (and HTTPS Certificates), then retry"),
                &remaining_after(4),
            );
        }
        ctx.done(&tr("MagicDNS enabled"));
    }

    // —— 5. proxy：启动 loopback 反代 ——
    {
        let ctx = StepCtx { app: &app, index: 5, id: steps[5] };
        if port_listening(PROXY_PORT) {
            ctx.done(&tr("Loopback proxy is running on 127.0.0.1:3898"));
        } else {
            let proxy_path = match ensure_proxy_script() {
                Ok(p) => p,
                Err(e) => return ctx.fail(&e, &tr("Cannot write ~/.dsh/loopback-proxy.js; check disk permissions"), &remaining_after(5)),
            };
            let node = match resolve_node_bin() {
                Ok(n) => n,
                Err(e) => return ctx.fail(&e, &tr("Install Node.js 18+ first"), &remaining_after(5)),
            };
            let log = dsh_dir().map(|d| d.join("loopback-proxy.log")).unwrap_or_else(|_| PathBuf::from("loopback-proxy.log"));
            ctx.running(&tr("Starting loopback proxy…"));
            let proxy_path_str = proxy_path.display().to_string();
            if let Err(e) = spawn_detached(
                &node,
                &[&proxy_path_str],
                &[],
                &log,
            ) {
                return ctx.fail(
                    &e,
                    &tr("Port 3898 may be occupied; stop the process using it and retry"),
                    &remaining_after(5),
                );
            }
            if !wait_port(PROXY_PORT, Duration::from_secs(10)) {
                return ctx.fail(
                    &tr("Loopback proxy did not start within 10s"),
                    &trf("Check the log at ~/.dsh/loopback-proxy.log", &[]),
                    &remaining_after(5),
                );
            }
            ctx.done(&tr("Loopback proxy is running on 127.0.0.1:3898"));
        }
    }

    // —— 6. serve：配置 Tailscale HTTPS serve ——
    {
        let ctx = StepCtx { app: &app, index: 6, id: steps[6] };
        let Some(ts) = tailscale_path() else {
            return ctx.fail(
                &tr("Tailscale is not installed"),
                &tr("Install Tailscale first, then retry"),
                &remaining_after(6),
            );
        };
        ctx.running(&tr("Configuring tailscale serve…"));
        if serve_configured(&ts) {
            let (_, url) = resolve_host_and_url();
            match url {
                Some(u) => ctx.done(&trf("HTTPS serve ready: {url}", &[("url", u)])),
                None => ctx.done(&tr("HTTPS serve ready")),
            }
        } else {
            let r = run_capture(&ts, &["serve", "--https=443", "--bg", &PROXY_PORT.to_string()]);
            match r {
                Ok((_, _, true)) => {
                    let (_, url) = resolve_host_and_url();
                    match url {
                        Some(u) => ctx.done(&trf("HTTPS serve ready: {url}", &[("url", u)])),
                        None => ctx.done(&tr("HTTPS serve ready")),
                    }
                }
                Ok((_, err, _)) => {
                    let e = if err.is_empty() { "tailscale serve failed".to_string() } else { err };
                    return ctx.fail(
                        &trf("Serve is not enabled or failed: {error}", &[("error", e)]),
                        &tr("Open the authorization link in the error output to enable Serve for this tailnet (https://login.tailscale.com/f/serve), then retry"),
                        &remaining_after(6),
                    )
                }
                Err(e) => {
                    return ctx.fail(
                        &e,
                        &tr("Run `tailscale up` first to sign in, then retry"),
                        &remaining_after(6),
                    )
                }
            }
        }
    }

    // —— 7. verify：验证远程访问链路 ——
    {
        let ctx = StepCtx { app: &app, index: 7, id: steps[7] };
        let (_, url) = resolve_host_and_url();
        let url_text = url.clone().unwrap_or_else(|| "https://<hostname>.ts.net".to_string());
        ctx.running(&trf("Verifying remote access ({url})…", &[("url", url_text.clone())]));
        let web_ok = http_ok(http_get(WEB_PORT, "127.0.0.1", "/").as_deref());
        let proxy_ok = http_ok(http_get(PROXY_PORT, "127.0.0.1", "/").as_deref());
        let serve_ok = tailscale_path()
            .map(|ts| serve_configured(&ts))
            .unwrap_or(false);
        if web_ok && proxy_ok && serve_ok {
            ctx.done(&trf("Remote access is ready: {url}", &[("url", url_text.clone())]));
        } else {
            let mut checks: Vec<String> = Vec::new();
            if !web_ok {
                checks.push(tr("dsh web is not responding (curl http://127.0.0.1:3899/)"));
            }
            if !proxy_ok {
                checks.push(tr("loopback proxy is not responding (curl http://127.0.0.1:3898/)"));
            }
            if !serve_ok {
                checks.push(tr("tailscale serve is not configured (tailscale serve status)"));
            }
            return ctx.fail(
                &tr("Verification failed; some components are not ready"),
                &checks.join("；"),
                &remaining_after(7),
            );
        }
    }

    Ok(())
}

// ============ 停止 ============

#[tauri::command]
pub fn dsh_stop() -> Result<(), String> {
    kill_by_pattern("dsh --profile web");
    kill_by_pattern(PROXY_SCRIPT);
    Ok(())
}

// ============ 开机自启（launchd / schtasks / systemd --user） ============

/// sh 单引号转义（生成的启动脚本内嵌绝对路径）
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 端口占用守卫脚本（node 一段式）：端口已被监听 → exit 0（自启服务不重复启动）
fn port_guard_js(port: u16) -> String {
    format!(
        "const net=require('net');const s=net.connect({port},'127.0.0.1');s.on('connect',()=>process.exit(0));s.on('error',()=>process.exit(1));setTimeout(()=>process.exit(1),1000).unref();",
        port = port
    )
}

/// 生成 dsh web 启动脚本（自启用）：带端口守卫 + SSH_CONNECTION + --trusted-host
fn render_start_web(node: &str, dsh: &str, host: &str) -> String {
    let trusted = if host.is_empty() {
        String::new()
    } else {
        format!(" --trusted-host {}", sh_quote(host))
    };
    format!(
        "#!/bin/sh\n# generated by Codex Pro Max; do not edit\nif {node} -e {guard}; then exit 0; fi\nexport SSH_CONNECTION={ssh}\nexec {dsh} --profile web --port {port}{trusted}\n",
        node = sh_quote(node),
        guard = sh_quote(&port_guard_js(WEB_PORT)),
        ssh = sh_quote(SSH_CONNECTION_ENV),
        dsh = sh_quote(dsh),
        port = WEB_PORT,
        trusted = trusted,
    )
}

/// 生成 loopback 反代启动脚本（自启用）：带端口守卫
fn render_start_proxy(node: &str, proxy: &str) -> String {
    format!(
        "#!/bin/sh\n# generated by Codex Pro Max; do not edit\nif {node} -e {guard}; then exit 0; fi\nexec {node} {proxy}\n",
        node = sh_quote(node),
        guard = sh_quote(&port_guard_js(PROXY_PORT)),
        proxy = sh_quote(proxy),
    )
}

/// XML 转义（plist 内容）
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn autostart_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        config::home_dir()
            .map(|h| h.join("Library/LaunchAgents").join(format!("{}.web.plist", AUTOSTART_PREFIX)).exists())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        // schtasks 查询成功（退出码 0）即任务存在
        matches!(
            Command::new("schtasks")
                .args(["/Query", "/TN", &format!("{}Web", AUTOSTART_PREFIX)])
                .output(),
            Ok(o) if o.status.success()
        )
    }
    #[cfg(target_os = "linux")]
    {
        config::home_dir()
            .map(|h| h.join(".config/systemd/user").join("dsh-remote-web.service").exists())
            .unwrap_or(false)
    }
}

#[tauri::command]
pub fn dsh_set_autostart(enabled: bool) -> Result<(), String> {
    autostart_impl(enabled)
}

#[cfg(target_os = "macos")]
fn autostart_impl(enabled: bool) -> Result<(), String> {
    let home = config::home_dir()?;
    let agents_dir = home.join("Library/LaunchAgents");
    let dsh = dsh_dir()?;
    let web_plist = agents_dir.join(format!("{}.web.plist", AUTOSTART_PREFIX));
    let proxy_plist = agents_dir.join(format!("{}.proxy.plist", AUTOSTART_PREFIX));
    let web_script = dsh.join("start-web.sh");
    let proxy_script = dsh.join("start-proxy.sh");

    if enabled {
        let node = resolve_node_bin()?;
        let dsh_bin = resolve_dsh_bin()?;
        let fqdn = resolve_fqdn().unwrap_or_default();
        fs::create_dir_all(&agents_dir)
            .map_err(|e| trf("Failed to create directory: {error}", &[("error", e.to_string())]))?;
        fs::write(&web_script, render_start_web(&node, &dsh_bin.display().to_string(), &fqdn))
            .map_err(|e| trf("Failed to write {path}: {error}", &[("path", web_script.display().to_string()), ("error", e.to_string())]))?;
        fs::write(&proxy_script, render_start_proxy(&node, &dsh.join(PROXY_SCRIPT).display().to_string()))
            .map_err(|e| trf("Failed to write {path}: {error}", &[("path", proxy_script.display().to_string()), ("error", e.to_string())]))?;

        let plist = |label: &str, script: &Path, log_file: &Path| -> String {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string>
        <string>{script}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
                label = label,
                script = xml_escape(&script.display().to_string()),
                home = xml_escape(&home.display().to_string()),
                log = xml_escape(&log_file.display().to_string()),
            )
        };
        fs::write(&web_plist, plist(&format!("{}.web", AUTOSTART_PREFIX), &web_script, &dsh.join("launchd-web.log")))
            .map_err(|e| trf("Failed to write {path}: {error}", &[("path", web_plist.display().to_string()), ("error", e.to_string())]))?;
        fs::write(&proxy_plist, plist(&format!("{}.proxy", AUTOSTART_PREFIX), &proxy_script, &dsh.join("launchd-proxy.log")))
            .map_err(|e| trf("Failed to write {path}: {error}", &[("path", proxy_plist.display().to_string()), ("error", e.to_string())]))?;

        let load = |plist: &Path| {
            Command::new("launchctl")
                .args(["load", "-w"])
                .arg(plist)
                .output()
        };
        if let Err(e) = load(&web_plist) {
            return Err(trf("Cannot register launchd agent: {error}", &[("error", e.to_string())]));
        }
        if let Err(e) = load(&proxy_plist) {
            return Err(trf("Cannot register launchd agent: {error}", &[("error", e.to_string())]));
        }
    } else {
        let unload = |label: &str, plist: &Path| {
            // 先 unload（忽略不存在错误），再删文件
            let _ = Command::new("launchctl").args(["unload", "-w"]).arg(plist).output();
            let _ = fs::remove_file(plist);
            let _ = Command::new("launchctl").arg("remove").arg(label).output();
        };
        unload(&format!("{}.web", AUTOSTART_PREFIX), &web_plist);
        unload(&format!("{}.proxy", AUTOSTART_PREFIX), &proxy_plist);
        let _ = fs::remove_file(&web_script);
        let _ = fs::remove_file(&proxy_script);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn autostart_impl(enabled: bool) -> Result<(), String> {
    let dsh = dsh_dir()?;
    fs::create_dir_all(&dsh)
        .map_err(|e| trf("Failed to create directory: {error}", &[("error", e.to_string())]))?;
    let web_cmd = dsh.join("start-web.cmd");
    let proxy_cmd = dsh.join("start-proxy.cmd");
    let task_web = format!("{}Web", AUTOSTART_PREFIX);
    let task_proxy = format!("{}Proxy", AUTOSTART_PREFIX);

    if enabled {
        let node = resolve_node_bin()?;
        let dsh_bin = resolve_dsh_bin()?;
        let fqdn = resolve_fqdn().unwrap_or_default();
        let trusted = if fqdn.is_empty() { String::new() } else { format!(" --trusted-host {}", fqdn) };
        let web = format!(
            "@echo off\r\nrem generated by Codex Pro Max; do not edit\r\n\"{node}\" -e \"{guard}\" >nul 2>&1\r\nif %errorlevel%==0 exit /b 0\r\nset SSH_CONNECTION={ssh}\r\n\"{dsh}\" --profile web --port {port}{trusted}\r\n",
            node = node, guard = port_guard_js(WEB_PORT), ssh = SSH_CONNECTION_ENV,
            dsh = dsh_bin.display().to_string(), port = WEB_PORT, trusted = trusted,
        );
        let proxy = format!(
            "@echo off\r\nrem generated by Codex Pro Max; do not edit\r\n\"{node}\" -e \"{guard}\" >nul 2>&1\r\nif %errorlevel%==0 exit /b 0\r\n\"{node}\" \"{proxy}\"\r\n",
            node = node, guard = port_guard_js(PROXY_PORT), proxy = dsh.join(PROXY_SCRIPT).display().to_string(),
        );
        fs::write(&web_cmd, web).map_err(|e| trf("Failed to write {path}: {error}", &[("path", web_cmd.display().to_string()), ("error", e.to_string())]))?;
        fs::write(&proxy_cmd, proxy).map_err(|e| trf("Failed to write {path}: {error}", &[("path", proxy_cmd.display().to_string()), ("error", e.to_string())]))?;

        let create = |task: &str, cmd: &Path| {
            Command::new("schtasks")
                .args(["/Create", "/F", "/TN", task, "/SC", "ONLOGON", "/RL", "LIMITED", "/TR"])
                .arg(format!("\"{}\"", cmd.display()))
                .output()
        };
        if let Err(e) = create(&task_web, &web_cmd) {
            return Err(trf("Cannot create scheduled task: {error}", &[("error", e.to_string())]));
        }
        if let Err(e) = create(&task_proxy, &proxy_cmd) {
            return Err(trf("Cannot create scheduled task: {error}", &[("error", e.to_string())]));
        }
    } else {
        for task in [&task_web, &task_proxy] {
            let _ = Command::new("schtasks").args(["/Delete", "/F", "/TN", task]).output();
        }
        let _ = fs::remove_file(&web_cmd);
        let _ = fs::remove_file(&proxy_cmd);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn autostart_impl(enabled: bool) -> Result<(), String> {
    let home = config::home_dir()?;
    let units_dir = home.join(".config/systemd/user");
    let dsh = dsh_dir()?;
    let web_unit = units_dir.join("dsh-remote-web.service");
    let proxy_unit = units_dir.join("dsh-remote-proxy.service");

    if enabled {
        let node = resolve_node_bin()?;
        let dsh_bin = resolve_dsh_bin()?;
        let fqdn = resolve_fqdn().unwrap_or_default();
        fs::create_dir_all(&units_dir)
            .map_err(|e| trf("Failed to create directory: {error}", &[("error", e.to_string())]))?;
        let guard_pre = |port: u16| -> String {
            format!(
                "{node} -e {guard} && exit 1 || exit 0",
                node = sh_quote(&node),
                guard = sh_quote(&port_guard_js(port)),
            )
        };
        let web = format!(
            "[Unit]\nDescription=DeepSeek Harness web (remote access)\nAfter=network.target\n\n[Service]\nType=simple\nExecStartPre=/bin/sh -c {pre}\nEnvironment=SSH_CONNECTION={ssh}\nExecStart={dsh} --profile web --port {port}{trusted}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            pre = sh_quote(&guard_pre(WEB_PORT)),
            ssh = sh_quote(SSH_CONNECTION_ENV),
            dsh = sh_quote(&dsh_bin.display().to_string()),
            port = WEB_PORT,
            trusted = if fqdn.is_empty() { String::new() } else { format!(" --trusted-host {}", sh_quote(&fqdn)) },
        );
        let proxy = format!(
            "[Unit]\nDescription=DeepSeek Harness loopback proxy (remote access)\nAfter=network.target\n\n[Service]\nType=simple\nExecStartPre=/bin/sh -c {pre}\nExecStart={node} {proxy}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            pre = sh_quote(&guard_pre(PROXY_PORT)),
            node = sh_quote(&node),
            proxy = sh_quote(&dsh.join(PROXY_SCRIPT).display().to_string()),
        );
        fs::write(&web_unit, web).map_err(|e| trf("Failed to write {path}: {error}", &[("path", web_unit.display().to_string()), ("error", e.to_string())]))?;
        fs::write(&proxy_unit, proxy).map_err(|e| trf("Failed to write {path}: {error}", &[("path", proxy_unit.display().to_string()), ("error", e.to_string())]))?;

        let sysctl = |args: &[&str]| {
            Command::new("systemctl").args(["--user"]).args(args).output()
        };
        let _ = sysctl(&["daemon-reload"]);
        if let Err(e) = sysctl(&["enable", "dsh-remote-web.service"]) {
            return Err(trf("Cannot enable systemd unit: {error}", &[("error", e.to_string())]));
        }
        if let Err(e) = sysctl(&["enable", "dsh-remote-proxy.service"]) {
            return Err(trf("Cannot enable systemd unit: {error}", &[("error", e.to_string())]));
        }
    } else {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "dsh-remote-web.service", "dsh-remote-proxy.service"])
            .output();
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
        let _ = fs::remove_file(&web_unit);
        let _ = fs::remove_file(&proxy_unit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{port_guard_js, render_start_proxy, render_start_web, sh_quote};

    #[test]
    fn sh_quote_handles_spaces_and_quotes() {
        assert_eq!(sh_quote("/Users/a b/node"), "'/Users/a b/node'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("/usr/local/bin/node"), "'/usr/local/bin/node'");
    }

    #[test]
    fn start_scripts_embed_guard_and_exec() {
        let web = render_start_web("/usr/local/bin/node", "/home/u/.npm-global/bin/dsh", "etmacmini.ts.net");
        assert!(web.contains("net.connect(3899"));
        assert!(web.contains("SSH_CONNECTION"));
        assert!(web.contains("--trusted-host 'etmacmini.ts.net'"));
        assert!(web.contains("exec '/home/u/.npm-global/bin/dsh' --profile web --port 3899"));
        let proxy = render_start_proxy("/usr/local/bin/node", "/home/u/.dsh/loopback-proxy.js");
        assert!(proxy.contains("net.connect(3898"));
        assert!(proxy.contains("exec '/usr/local/bin/node' '/home/u/.dsh/loopback-proxy.js'"));
    }

    #[test]
    fn guard_js_targets_loopback() {
        assert!(port_guard_js(3899).contains("net.connect(3899,'127.0.0.1')"));
    }
}
