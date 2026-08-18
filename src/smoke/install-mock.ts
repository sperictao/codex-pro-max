// 冒烟 Mock：在浏览器里模拟 Tauri 运行时与 Rust 后端（window.__TAURI_INTERNALS__）。
// 覆盖全部 commands/events/插件调用；带可写状态，使启停、看守、语言切换等流程可在无后端下闭环。
// 该文件仅被 smoke.html 引用，不进生产 bundle。

/* eslint-disable @typescript-eslint/no-explicit-any */

type AnyRec = Record<string, any>;

const cbMap = new Map<number, (payload: AnyRec) => void>();
let nextCb = 1;
const eventListeners = new Map<string, number[]>();

// ============ Fixture 状态 ============
const config: AnyRec = {
  taskboard_path: "/opt/dashi-taskboard",
  node_path: "",
  codex_app_path: "/Applications/ChatGPT.app",
  taskboard_host: "127.0.0.1",
  taskboard_port: 47823,
  cdp_port: 9231,
  auto_open: true,
  separate_window_mode: false,
  minimize_to_tray_on_close: true,
  language: "system",
  codex_guard: { enabled: true, params: {} },
};
let resolvedLanguage = "en";

const processes: AnyRec[] = [
  { name: "taskboard-server", status: "stopped", pid: null, message: "" },
  { name: "injector", status: "stopped", pid: null, message: "" },
];

const guardParams: AnyRec[] = [
  {
    id: "features.image_generation", label: "Image Generation", description: "Enable image generation",
    applyMode: "toml_key", valueType: "bool", path: "features.image_generation",
    default: true, value: false, applied: true, locked: false,
    actual: "false", status: "match", error: null, lastChecked: null, lastRestored: null, custom: false,
  },
  {
    id: "features.multi_agent_v2.enabled", label: "Multi Agent V2", description: "",
    applyMode: "toml_key", valueType: "int", path: "features.multi_agent_v2.enabled",
    default: 60, value: 42, applied: true, locked: true,
    actual: "42", status: "match", error: null, lastChecked: 1760000000, lastRestored: 1759990000, custom: false,
  },
  {
    id: "model_reasoning", label: "Reasoning Effort", description: "Reasoning level",
    applyMode: "toml_key", valueType: "string", path: "model_reasoning",
    default: "high", value: "medium", applied: false, locked: false,
    actual: "low", status: "drift", error: null, lastChecked: null, lastRestored: null, custom: false,
  },
  {
    id: "agents_md_block", label: "AGENTS.md Block", description: "",
    applyMode: "markdown_block", valueType: "text", path: "",
    default: "line1\nline2", value: "line1\nline2", applied: true, locked: false,
    actual: "line1\\nline2", status: "match", error: null, lastChecked: null, lastRestored: null, custom: false,
  },
  {
    id: "agents_section", label: "Agents Section (absent)", description: "",
    applyMode: "toml_absent", valueType: "none", path: "agents",
    default: null, value: null, applied: true, locked: false,
    actual: "absent", status: "match", error: null, lastChecked: null, lastRestored: null, custom: false,
  },
  {
    id: "custom.my_flag", label: "My Flag", description: "user defined",
    applyMode: "toml_key", valueType: "bool", path: "custom.my_flag",
    default: false, value: true, applied: true, locked: false,
    actual: "true", status: "match", error: null, lastChecked: null, lastRestored: null, custom: true,
  },
];

const guardView: AnyRec = {
  enabled: true,
  groups: [
    { id: "g-config", name: "config.toml", file: "config.toml", format: "toml", builtin: true, error: null, params: guardParams.slice(0, 4) },
    { id: "g-agents", name: "agents/default.toml", file: "agents/default.toml", format: "toml", builtin: true, error: null, params: guardParams.slice(4, 5) },
    { id: "g-extra", name: "extra.toml", file: "extra.toml", format: "toml", builtin: false, error: null, params: guardParams.slice(5) },
  ],
};

let guardFiles: AnyRec[] = [
  { id: "g-config", name: "config.toml", file: "config.toml", format: "toml", builtin: true, detection: { path: "config.toml", at: 1760000000 } },
  { id: "g-agents", name: "agents/default.toml", file: "agents/default.toml", format: "toml", builtin: true, detection: null },
  { id: "g-extra", name: "extra.toml", file: "extra.toml", format: "toml", builtin: false, detection: null },
];

let fastctx: AnyRec = { installed: true, version: "1.2.3", integrated: false, latestVersion: "1.3.0" };

const dsh: AnyRec = {
  nodeAvailable: true, dshInstalled: true, dshVersion: "0.1.0-rc.6",
  supportedVersion: "0.1.0-rc.6", dshCompatible: true, pluginsInstalled: true,
  dshRunning: false, tailscaleInstalled: true, tailscaleOnline: true,
  hostname: "mbp", localUrl: null, url: null, magicDnsEnabled: true,
  serveConfigured: false,
  autostartEnabled: false, error: null,
};

let autostart = false;
let updateInfo: AnyRec = {
  currentVersion: "1.2.0-smoke", availableVersion: "1.3.0", hasUpdate: true,
  releaseNotes: "- 看守视图改进\n- 修复若干问题", message: null,
};

function findParam(id: string): AnyRec | undefined {
  return guardView.groups.flatMap((g: AnyRec) => g.params).find((p: AnyRec) => p.id === id);
}
function setProc(name: string, status: string): void {
  const p = processes.find((x) => x.name === name);
  if (p) p.status = status;
}

// ============ 命令路由 ============
const routes: Record<string, (args: AnyRec) => any> = {
  get_resolved_language: () => resolvedLanguage,
  set_language: ({ setting }) => {
    config.language = setting;
    resolvedLanguage = setting === "system" ? "en" : setting;
  },
  load_config: () => structuredClone(config),
  update_settings: ({ config: c }) => Object.assign(config, c),
  get_bundled_taskboard_path: () => "/bundled/dashi-taskboard",
  autostart_is_enabled: () => autostart,
  autostart_set: ({ enabled }) => { autostart = enabled; },
  check_node_version: () => "v22.11.0",
  check_codex_app: () => true,
  detect_codex_app: () => "/Applications/ChatGPT.app",
  validate_taskboard_path: () => true,
  get_log_dir: () => "/tmp/logs",

  get_status: () => structuredClone(processes),
  start_all: () => { setProc("taskboard-server", "running"); setProc("injector", "running"); },
  stop_all: () => { setProc("taskboard-server", "stopped"); setProc("injector", "stopped"); },
  start_taskboard: () => setProc("taskboard-server", "running"),
  stop_taskboard: () => setProc("taskboard-server", "stopped"),
  start_injector: () => setProc("injector", "running"),
  stop_injector: () => setProc("injector", "stopped"),
  open_taskboard: () => null,
  quit_codex: () => null,

  guard_get_view: () => structuredClone(guardView),
  guard_set_enabled: ({ enabled }) => { guardView.enabled = enabled; config.codex_guard.enabled = enabled; },
  guard_set_value: ({ id, value }) => { const p = findParam(id); if (p) p.value = value; },
  guard_apply: ({ id }) => {
    const p = findParam(id);
    if (p) { p.applied = true; p.status = "match"; p.actual = String(p.value ?? p.default ?? ""); }
  },
  guard_set_applied: ({ id, applied }) => { const p = findParam(id); if (p) p.applied = applied; },
  guard_set_locked: ({ id, locked }) => {
    const p = findParam(id);
    if (p) { p.locked = locked; p.lastChecked = 1760001000; p.lastRestored = locked ? 1760001000 : null; }
  },
  guard_add_custom_param: ({ param, fileId }) => {
    const g = guardView.groups.find((x: AnyRec) => x.id === fileId);
    if (g) g.params.push({ ...param, value: param.default, applied: false, locked: false, actual: null, status: "missing", error: null, lastChecked: null, lastRestored: null });
  },
  guard_remove_custom_param: ({ id }) => {
    for (const g of guardView.groups) g.params = g.params.filter((p: AnyRec) => p.id !== id);
  },
  guard_get_files: () => structuredClone(guardFiles),
  guard_detect_file: ({ id }) => {
    const f = guardFiles.find((x) => x.id === id);
    if (f) f.detection = { path: f.file, at: 1760002000 };
    return structuredClone(f);
  },
  guard_update_file: ({ id, name, file }) => {
    const f = guardFiles.find((x) => x.id === id);
    if (f) { f.name = name; f.file = file; }
  },
  guard_add_file: ({ name, file, format }) => {
    guardFiles.push({ id: `g-${file}`, name, file, format, builtin: false, detection: null });
  },
  guard_remove_file: ({ id }) => { guardFiles = guardFiles.filter((x) => x.id !== id); },
  guard_get_schema_file_path: () => "/tmp/schema.json",
  guard_relativize_picked_path: ({ absPath }) => String(absPath).replace(/^.*\.codex\//, ""),

  check_skill_status: () => ({ state: "installed", detail: "Symlink 指向 /opt/dashi-taskboard", targetPath: "/opt/dashi-taskboard" }),
  install_skill: () => "Installed to ~/.codex/skills/manage-taskboard",

  fastctx_detect: () => structuredClone(fastctx),
  fastctx_install: () => { fastctx.installed = true; },
  fastctx_apply: () => { fastctx.integrated = true; return { selfCheckPassed: true, selfCheckOutput: "[OK] all" }; },
  fastctx_unapply: () => { fastctx.integrated = false; },
  fastctx_open_console: () => null,

  dsh_detect: () => structuredClone(dsh),
  dsh_setup: () => {
    Object.assign(dsh, { dshRunning: true, serveConfigured: true, url: "https://mbp.ts.net" });
  },
  dsh_start_web: () => {
    Object.assign(dsh, { dshRunning: true, localUrl: "http://127.0.0.1:3899" });
    return "http://127.0.0.1:3899";
  },
  dsh_stop: () => { Object.assign(dsh, { dshRunning: false, serveConfigured: false, url: null, localUrl: null }); },
  dsh_update: () => {
    Object.assign(dsh, { dshVersion: "0.1.0-rc.6", dshCompatible: true, pluginsInstalled: true });
    return "0.1.0-rc.6";
  },
  dsh_set_autostart: ({ enabled }) => { dsh.autostartEnabled = enabled; },

  get_updater_config_health: () => ({ configured: true, message: "" }),
  get_updater_help_paths: () => ({ docsPath: "https://docs", templatePath: "https://tpl" }),
  check_update: () => structuredClone(updateInfo),
  install_update: () => { updateInfo = { ...updateInfo, hasUpdate: false }; return "Updated to v1.3.0"; },

  "plugin:app|version": () => "1.2.0-smoke",
  "plugin:notification|is_permission_granted": () => true,
  "plugin:notification|request_permission": () => "granted",
  "plugin:dialog|message": () => ((window as any).__smoke?.askAnswer === false ? "No" : "Yes"),
  "plugin:dialog|open": () => "/Users/me/.codex/picked.toml",
  "plugin:shell|open": (args) => {
    (window as any).__smoke?.calls.push({ cmd: "shell:open", args });
    return null;
  },
  "plugin:event|listen": ({ event, handler }) => {
    const arr = eventListeners.get(event) ?? [];
    arr.push(handler);
    eventListeners.set(event, arr);
    return handler;
  },
  "plugin:event|unlisten": () => null,
};

// ============ 安装 __TAURI_INTERNALS__ ============
const w = window as any;
w.__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args?: AnyRec) => {
    const fn = routes[cmd];
    if (!fn) {
      console.error(`[smoke-mock] unknown command: ${cmd}`, args);
      return Promise.reject(new Error(`smoke mock: unknown command ${cmd}`));
    }
    try {
      return Promise.resolve(structuredClone(fn(args ?? {})) ?? null);
    } catch (e) {
      return Promise.reject(e);
    }
  },
  transformCallback: (cb: (p: AnyRec) => void) => {
    const id = nextCb++;
    cbMap.set(id, cb);
    return id;
  },
  unregisterCallback: (id: number) => cbMap.delete(id),
  convertFileSrc: (p: string) => p,
  metadata: {},
  plugins: {},
};
w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };

// 驱动脚本挂钩
w.__smoke = {
  askAnswer: true,
  calls: [] as AnyRec[],
  emit(event: string, payload: AnyRec) {
    for (const id of eventListeners.get(event) ?? []) cbMap.get(id)?.({ event, id: 0, payload });
  },
};
