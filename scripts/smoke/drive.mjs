// 冒烟驱动：playwright-core + headless chromium，逐项核对 docs/react-migration-plan.md §4
// 用法：pnpm smoke（dev server 未运行时会自动拉起并在结束后关闭）
// 全新机器先执行：pnpm exec playwright install chromium-headless-shell
import { chromium } from "playwright-core";
import fs from "node:fs";
import { spawn } from "node:child_process";

const SHOTS = "/tmp/tauri-smoke/shots";
const URL = "http://localhost:5173/smoke.html";
const results = [];
const consoleErrors = [];

function check(name, cond, extra = "") {
  results.push({ name, pass: !!cond, extra });
  console.log(`${cond ? "PASS" : "FAIL"}  ${name}${extra ? ` — ${extra}` : ""}`);
}

// dev server 自拉起（已在跑则复用）
async function ensureDevServer() {
  try {
    await fetch("http://localhost:5173/smoke.html", { signal: AbortSignal.timeout(1500) });
    return null;
  } catch {
    const child = spawn("pnpm", ["dev"], { stdio: "ignore", detached: false });
    for (let i = 0; i < 30; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      try {
        await fetch("http://localhost:5173/smoke.html", { signal: AbortSignal.timeout(1000) });
        return child;
      } catch { /* 继续等 */ }
    }
    child.kill();
    throw new Error("dev server 启动超时");
  }
}
const devServer = await ensureDevServer();

// 浏览器解析：优先 playwright registry 默认路径，失败回落缓存里最新的 headless shell
async function launch() {
  try {
    return await chromium.launch();
  } catch {
    const cache = process.env.HOME + "/Library/Caches/ms-playwright";
    const shells = fs.readdirSync(cache).filter((d) => d.startsWith("chromium_headless_shell-")).sort();
    const latest = shells[shells.length - 1];
    if (!latest) throw new Error("无可用 Chromium，请运行 pnpm exec playwright install chromium-headless-shell");
    return chromium.launch({
      executablePath: `${cache}/${latest}/chrome-headless-shell-mac-arm64/chrome-headless-shell`,
    });
  }
}
const browser = await launch();
const page = await (await browser.newContext({ viewport: { width: 1100, height: 800 } })).newPage();
page.on("console", (m) => { if (m.type() === "error") consoleErrors.push(m.text()); });
page.on("pageerror", (e) => consoleErrors.push(String(e)));

const shot = async (name) => page.screenshot({ path: `${SHOTS}/${name}.png`, fullPage: false });
const txt = async (sel) => page.locator(sel).first().textContent().catch(() => null);
const visible = async (sel) => page.locator(sel).first().isVisible().catch(() => false);
const dataTheme = () => page.evaluate(() => document.documentElement.dataset.theme);

// ============ 启动序列 ============
await page.goto(URL);
await page.waitForSelector("text=Codex Pro Max", { timeout: 15000 });
await page.waitForTimeout(800); // 等 init（load_config/health/silent check）完成
check("启动：页面渲染，header 出现", await visible("header"));
check("启动：无初始化失败 toast", !(await visible(".toast.error")));
check("启动：主题已应用（data-theme=vercel-light）", (await dataTheme()) === "vercel-light", await dataTheme());
check("启动：进程卡初始为 Stopped ×2", (await page.locator(".status-badge.stopped").count()) >= 2);
check("启动：消息行渲染 ×2", (await page.locator("#main-view .min-h-8").count()) === 2);
await shot("01-home-boot");

// Start All → running
await page.click("#btn-start-all");
await page.waitForSelector(".toast:has-text('All services started')");
check("主页：Start All 成功 toast", true);
await page.waitForTimeout(300);
check("主页：两卡 Running", (await page.locator(".status-badge.running").count()) >= 2);
check("主页：总指示器 All services running", (await txt(".status-indicator-text")) === "All services running");
check("主页：Start All 禁用 / Stop All 可用", await page.locator("#btn-start-all").isDisabled() && !(await page.locator("#btn-stop-all").isDisabled()));
check("主页：Open（taskboard running 后可用）", !(await page.locator("#main-view button:has-text('Open')").isDisabled()));
await shot("02-home-running");

// 事件桥：status-update 推送
await page.evaluate(() => window.__smoke.emit("status-update", { name: "taskboard-server", status: "failed", message: "boom" }));
await page.waitForTimeout(200);
check("事件桥：status-update → 卡变 Failed", (await page.locator(".status-badge.failed").count()) >= 1);
check("事件桥：总指示器 Service issue", (await txt(".status-indicator-text")) === "Service issue");
await page.evaluate(() => window.__smoke.emit("status-update", { name: "taskboard-server", status: "running", message: "" }));
await page.waitForTimeout(200);

// Stop All → stopped
await page.click("#btn-stop-all");
await page.waitForSelector(".toast:has-text('All services stopped')");
check("主页：Stop All 成功 toast + 指示器 Services stopped", (await txt(".status-indicator-text")) === "Services stopped");

// ============ 设置-通用 ============
await page.click("header button:has-text('Settings')");
await page.waitForSelector("#section-general");
check("设置：通用分区展示", await visible("#section-general"));
check("设置：footer 在通用分区可见", await visible("#settings-footer"));
check("通用：taskboard 路径已填充", (await page.locator("#cfg-path").inputValue()) === "/opt/dashi-taskboard");
check("通用：校验 Valid", (await page.locator(".config-validate.ok").count()) >= 1);
check("通用：node 版本显示 v22.11.0", (await txt("#section-general"))?.includes("v22.11.0"));
check("通用：托盘开关勾选（fixture true）", await page.locator("#toggle-tray").isChecked());
check("通用：自启动未勾选（OS 注册项 false）", !(await page.locator("#toggle-autostart").isChecked()));
await shot("03-settings-general");

// 自启动开关即时生效
await page.click("#toggle-autostart");
await page.waitForTimeout(200);
check("通用：自启动切换保持勾选（mock 成功）", await page.locator("#toggle-autostart").isChecked());

// 语言切换 → 中文
await page.click("button:has-text('中文')");
await page.waitForTimeout(500);
check("i18n：切中文后导航显示「主页」", await visible("header button:has-text('主页')"));
check("i18n：设置标题中文化", (await page.locator("h2").first().textContent()) === "通用");
await shot("04-settings-zh");
await page.click("button:has-text('English')");
await page.waitForTimeout(500);
check("i18n：切回英文", await visible("header button:has-text('Home')"));

// ============ 设置-外观 ============
await page.click('.nav-item:has-text("Appearance")');
check("外观：41 族色板卡渲染", (await page.locator("#theme-family-grid button").count()) >= 40, String(await page.locator("#theme-family-grid button").count()));
check("外观：footer 在外观分区隐藏", !(await visible("#settings-footer")));
const before = await dataTheme();
await page.locator("#theme-family-grid button").nth(1).click();
await page.waitForTimeout(200);
const afterFamily = await dataTheme();
check("外观：切族后 data-theme 变化", before !== afterFamily && afterFamily.endsWith("-light"), `${before} → ${afterFamily}`);
await page.click("button:has-text('Dark')");
await page.waitForTimeout(200);
check("外观：暗模式 → data-theme 以 -dark 结尾", (await dataTheme()).endsWith("-dark"), await dataTheme());
await page.click("button:has-text('Follow System')");
await page.locator("#theme-family-grid button:has-text('Vercel')").click();
await page.waitForTimeout(200);
check("外观：还原 vercel + system", (await dataTheme()) === "vercel-light", await dataTheme());
await shot("05-appearance");

// ============ 设置-网络 / 模式 ============
await page.click('.nav-item:has-text("Network")');
check("网络：host/port/cdp 已填充", (await page.locator("#cfg-host").inputValue()) === "127.0.0.1" && (await page.locator("#cfg-port").inputValue()) === "47823");
await page.click('.nav-item:has-text("Mode")');
check("模式：启动模式标签（全量）", (await txt("#section-mode"))?.includes("Full launch mode (restarts Codex)"));
await page.click("#toggle-mode");
await page.waitForTimeout(200);
check("模式：切换后标签变为分离窗口模式", (await txt("#section-mode"))?.includes("Separate window mode (does not restart Codex)"));
await page.click("#toggle-mode");

// 保存
await page.click("#btn-save-config");
await page.waitForSelector(".toast:has-text('Settings saved')");
check("保存：Settings saved toast", true);

// ============ 设置-看守（文件管理） ============
await page.click('.nav-item:has-text("Guard")');
await page.waitForSelector("#settings-guard-files");
await page.waitForTimeout(500); // 自动检测内置文件
check("看守设置：footer 隐藏", !(await visible("#settings-footer")));
check("看守设置：3 个文件卡", (await page.locator("#settings-guard-files > div").count()) === 3);
check("看守设置：内置文件 Built-in 禁用", await page.locator("button:has-text('Built-in')").first().isDisabled());
check("看守设置：自定义文件有 Delete", await visible("button:has-text('Delete')"));
check("看守设置：检测记录文案（path matches）", (await txt("#settings-guard-files"))?.includes("Detection: path matches"));
await shot("06-settings-guard");

// Detect 手动
await page.click("button:has-text('Detect')");
await page.waitForSelector(".toast:has-text('Detection complete: path matches')");
check("看守设置：手动 Detect 一致 toast", true);

// Edit 弹窗：格式禁用
await page.click("button:has-text('Edit')");
await page.waitForSelector(".modal-overlay");
check("文件弹窗：编辑标题", await visible("text=Edit Guard File"));
check("文件弹窗：格式下拉禁用", await page.locator(".modal-overlay select").first().isDisabled());
await shot("07-file-modal-edit");
await page.click(".modal-overlay button:has-text('Cancel')");

// Add File + Pick…
await page.click("#guard-file-form-toggle");
await page.waitForSelector(".modal-overlay");
await page.click("button:has-text('Pick…')");
await page.waitForTimeout(300);
check("文件弹窗：Pick… 回填相对路径", (await page.locator(".modal-overlay input").nth(1).inputValue()) === "picked.toml", await page.locator(".modal-overlay input").nth(1).inputValue());
check("文件弹窗：名称自动带入", (await page.locator(".modal-overlay input").first().inputValue()) === "picked.toml");
await page.click(".modal-overlay button:has-text('Cancel')");

// 总开关关 → Guard Tab 隐藏；开 → 恢复
await page.click("#settings-guard-toggle");
await page.waitForSelector(".toast:has-text('Config guard disabled')");
check("看守设置：总开关关闭 toast", true);
check("看守设置：Guard Tab 隐藏", !(await visible("header button:has-text('Guard')")));
await page.click("#settings-guard-toggle");
await page.waitForSelector(".toast:has-text('Config guard enabled')");
check("看守设置：总开关开启后 Guard Tab 恢复", await visible("header button:has-text('Guard')"));

// ============ 看守视图 ============
await page.click("header button:has-text('Guard')");
await page.waitForSelector(".guard-param-card");
check("看守视图：3 分组渲染", (await page.locator("[data-group-id]").count()) === 3);
check("看守视图：参数卡数量 6", (await page.locator(".guard-param-card").count()) === 6);
check("看守视图：状态徽标含 Drift", await visible(".status-badge.failed:has-text('Drift')"));
check("看守视图：锁定参数时间行", (await txt("#guard-view"))?.includes("Last checked"));
check("看守视图：锁定参数编辑器禁用", await page.locator("[data-guard-id='features.multi_agent_v2.enabled']").isDisabled());
check("看守视图：未启用参数 Lock 禁用", (await page.locator("#guard-view button:has-text('Lock'):disabled").count()) >= 1);
const cardBtns = await page.locator(".guard-param-card").last().locator("button").allTextContents();
check("看守视图：自定义参数有 Delete", (await page.locator("#guard-view .guard-param-card button:has-text('Delete')").count()) >= 1, cardBtns.join("/"));
await shot("08-guard-view");

// bool 切换
await page.click("[data-guard-id='features.image_generation']");
await page.waitForTimeout(400);
check("看守视图：bool 切换后显示 true（取反落盘+刷新）", (await txt("#guard-view"))?.includes("true (recommended true)"));

// 锁定流程：未锁参数 Lock → Unlock 出现
const lockBtns = page.locator("button:has-text('Lock'):not([disabled])");
await lockBtns.first().click();
await page.waitForSelector(".toast:has-text('Locked')");
check("看守视图：锁定成功 toast + Unlock 按钮", await visible("button:has-text('Unlock')"));
// 解锁还原
await page.locator(".guard-param-card:has([data-guard-id='features.image_generation']) button:has-text('Unlock')").click();
await page.waitForSelector(".toast:has-text('Unlocked')");
check("看守视图：解锁成功 toast", true);

// 添加自定义参数弹窗联动
await page.click("#guard-add-toggle");
await page.waitForSelector(".modal-overlay");
const modalText = async () => (await page.locator(".modal-overlay").textContent()) ?? "";
check("添加弹窗：toml_key 显示 TOML Path", (await modalText()).includes("TOML Path"));
await page.selectOption(".modal-overlay select >> nth=0", "file_overwrite");
await page.waitForTimeout(200);
check("添加弹窗：file_overwrite 隐藏 TOML Path 与值类型", !(await modalText()).includes("TOML Path") && !(await modalText()).includes("Value Type"));
await page.selectOption(".modal-overlay select >> nth=0", "toml_key");
await page.waitForTimeout(200);
await page.selectOption(".modal-overlay select >> nth=2", "none");
await page.waitForTimeout(200);
check("添加弹窗：值类型 none 隐藏默认值行", !(await modalText()).includes("Default Value"));
await page.selectOption(".modal-overlay select >> nth=2", "text");
await page.waitForTimeout(200);
check("添加弹窗：text 类型默认值为 textarea", (await page.locator(".modal-overlay textarea").count()) === 1);
// 空 ID 提交 → 校验 toast
await page.click(".modal-overlay button:has-text('Add')");
await page.waitForSelector(".toast:has-text('Please enter an ID')");
check("添加弹窗：空 ID 校验 toast", true);
await shot("09-add-param-modal");
await page.click(".modal-overlay button:has-text('Cancel')");

// 成功添加一个自定义参数
await page.click("#guard-add-toggle");
await page.waitForSelector(".modal-overlay");
await page.fill(".modal-overlay input >> nth=0", "smoke_param");
await page.fill(".modal-overlay input >> nth=1", "Smoke Param");
await page.fill(".modal-overlay input >> nth=2", "features.smoke");
await page.click(".modal-overlay button:has-text('Add')");
await page.waitForSelector(".toast:has-text('Custom parameter added')");
check("添加弹窗：成功添加自定义参数", true);
await page.waitForTimeout(400);
check("看守视图：新参数出现在分组中", (await txt("#guard-view"))?.includes("Smoke Param"));

// ============ Skill ============
await page.click("header button:has-text('Skill')");
await page.waitForTimeout(400);
check("Skill：徽标 Installed", await visible("#skill-status-badge:has-text('Installed')"));
check("Skill：detail 显示", (await txt("#skill-view"))?.includes("Symlink"));
await shot("10-skill");

// ============ 集成 ============
await page.click("header button:has-text('Integrations')");
await page.waitForTimeout(600);
check("集成：dsh 状态链文案（web 未运行）", (await txt("#integration-view"))?.includes("dsh web not running"));
check("集成：dsh 版本胶囊", (await txt("#integration-view"))?.includes("0.1.0-rc.6"));
check("集成：fastctx 状态文案", (await txt("#integration-view"))?.includes("not integrated"));
check("集成：fastctx 更新胶囊 v1.3.0", (await txt("#integration-view"))?.includes("v1.3.0"));
check("集成：dsh 模式开关默认本地", !(await page.locator("#toggle-dsh-remote-access").isChecked()));
check("集成：dsh 时间轴（检测驱动，本地 4 步）", (await page.locator(".timeline-node").count()) === 4);
await shot("11-integration");

// 切换到远程模式：开关只是选择模式；时间轴切为远程 8 步
await page.click("#toggle-dsh-remote-access");
await page.waitForTimeout(400);
check("集成：切换后远程模式时间轴（8 步）", (await page.locator(".timeline-node").count()) === 8);

// 持久化：刷新后仍记住远程模式
await page.reload();
await page.waitForSelector("text=Codex Pro Max", { timeout: 15000 });
await page.click("header button:has-text('Integrations')");
await page.waitForTimeout(600);
check("集成：刷新后仍为远程模式", await page.locator("#toggle-dsh-remote-access").isChecked());
check("集成：刷新后远程时间轴（8 步）", (await page.locator(".timeline-node").count()) === 8);

// 远程模式一键启动（dsh_setup 全链路）→ 全 done + 远程 url 胶囊
await page.click("button:has-text('One-click start dsh web')");
await page.waitForSelector(".toast:has-text('Remote access ready')", { timeout: 8000 });
await page.waitForTimeout(500);
check("集成：dsh 远程启动成功 toast", true);
check("集成：远程模式 url 胶囊", (await txt("#integration-view"))?.includes("https://mbp.ts.net"));
check("集成：远程模式状态 Remote access ready", (await txt("#integration-view"))?.includes("Remote access ready"));
check("集成：远程模式时间轴全 done", (await page.locator(".timeline-node[data-state='done']").count()) === 8);
await shot("12-dsh-ready");

// 切换到本地模式：时间轴切回本地 4 步 + 本地 url 胶囊
await page.click("#toggle-dsh-remote-access");
await page.waitForTimeout(400);
check("集成：本地模式时间轴（4 步）", (await page.locator(".timeline-node").count()) === 4);
check("集成：本地模式 url 胶囊", (await txt("#integration-view"))?.includes("http://127.0.0.1:3899"));
check("集成：本地模式状态 Local access ready", (await txt("#integration-view"))?.includes("Local access ready"));
await shot("12b-dsh-local");

// 本地模式一键关闭 → 状态回未运行 + URL 胶囊（Copy/Open 按钮）消失
await page.click("button:has-text('One-click stop dsh web')");
await page.waitForSelector(".toast:has-text('dsh web stopped')");
await page.waitForTimeout(400);
check("集成：本地模式停止后 url 胶囊消失", !(await visible("#integration-view button:has-text('Copy')")));
check("集成：本地模式停止后状态 dsh web not running", (await txt("#integration-view"))?.includes("dsh web not running"));

// 切回远程模式并重新启动，供后续 dsh-step 事件桥用例（8 步时间轴）
await page.click("#toggle-dsh-remote-access");
await page.waitForTimeout(300);
check("集成：切回远程模式时间轴（8 步）", (await page.locator(".timeline-node").count()) === 8);
await page.click("button:has-text('One-click start dsh web')");
await page.waitForSelector(".toast:has-text('Remote access ready')", { timeout: 8000 });
await page.waitForTimeout(400);

// dsh-step 事件桥：失败节点问题+解决方案
await page.evaluate(() => window.__smoke.emit("dsh-step", { index: 7, id: "verify", state: "failed", detail: null, problem: "端口被占用", solution: "关闭 3899 占用进程" }));
await page.waitForTimeout(300);
check("事件桥：dsh-step 失败节点显示问题与解决方案", (await txt("#integration-view"))?.includes("端口被占用") && (await txt("#integration-view"))?.includes("关闭 3899 占用进程"));
await shot("13-dsh-step-failed");

// fastctx 接入
await page.click("#toggle-fastctx");
await page.waitForSelector(".toast:has-text('fastctx integrated')");
check("集成：fastctx 接入成功", (await txt("#integration-view"))?.includes("Integrated"));
// 摘除（mock ask=Yes）
await page.click("#toggle-fastctx");
await page.waitForSelector(".toast:has-text('fastctx unapplied')");
check("集成：fastctx 摘除成功（ask 确认后）", true);

// ============ 关于 / 更新 ============
await page.click("header button:has-text('Settings')");
await page.click('.nav-item:has-text("About")');
await page.waitForTimeout(300);
check("关于：版本号 1.2.0-smoke", (await txt("#about-version")) === "1.2.0-smoke");
check("关于：更新源 Ready", (await txt("#section-about"))?.includes("Ready"));
check("关于：帮助行隐藏（源健康）", !(await txt("#section-about"))?.includes("Configuration Help"));
check("关于：启动静默检查已发现更新 v1.3.0", (await txt("#section-about"))?.includes("v1.3.0"));
check("关于：release notes 显示", (await txt("#section-about"))?.includes("看守视图改进"));
check("关于：按钮文案 Update Now", (await txt("#btn-check-update")) === "Update Now");
await shot("14-about-update");

// 头部更新徽标 + 软件名链接
check("头部：检测到更新显示绿色下载按钮", await visible("[data-testid='update-badge']"));
check("头部：常态显示实体环", (await page.locator("[data-idle-ring]").count()) === 1);
check("头部：常态无进度弧", (await page.locator("[data-progress-ring]").count()) === 0);
check("头部：常态显示箭头（非数值）", (await page.locator("[data-testid='update-badge'] [data-arrow]").count()) === 1);
await page.click("header button:has-text('Codex Pro Max')");
await page.waitForTimeout(200);
const calls = await page.evaluate(() => window.__smoke.calls);
check("头部：软件名点击打开 GitHub 仓库", calls.some((c) => c.cmd === "shell:open" && String(c.args?.path ?? c.args ?? "").includes("sperictao/codex-pro-max")), JSON.stringify(calls));

// 下载进度事件桥
await page.evaluate(() => window.__smoke.emit("updater-download-progress", { stage: "downloading", version: "1.3.0", downloadedBytes: 0, totalBytes: 100, percent: 42.4, attempt: 1, maxAttempts: 3 }));
await page.waitForTimeout(200);
check("事件桥：下载进度行出现 42%", (await txt("#section-about"))?.includes("42%"));
const ringOffset = await page.locator("[data-progress-ring]").getAttribute("stroke-dashoffset");
const ringExpected = 2 * Math.PI * 9 * (1 - 0.424);
check("头部：进度环随百分比填充（stroke-dashoffset）", ringOffset !== null && Math.abs(parseFloat(ringOffset) - ringExpected) < 0.6, `${ringOffset} vs ${ringExpected.toFixed(2)}`);
check("头部：下载中实体环转为进度环", (await page.locator("[data-idle-ring]").count()) === 0);
check("头部：下载中箭头切换为百分比数值", (await page.locator("[data-testid='update-badge']").textContent())?.trim() === "42");
check("头部：下载中无箭头图标", (await page.locator("[data-testid='update-badge'] [data-arrow]").count()) === 0);
await shot("15-update-progress");

// Update Now → install
await page.click("[data-testid='update-badge']");
await page.waitForSelector(".toast:has-text('Updated to v1.3.0')");
check("头部：徽标点击即安装（成功 toast）", true);
check("头部：安装后更新徽标消失", !(await visible("[data-testid='update-badge']")));
check("关于：安装后更新行隐藏 + 按钮回 Check for Updates", !(await txt("#section-about"))?.includes("Available Update") && (await txt("#btn-check-update")) === "Check for Updates");
check("关于：安装后进度行隐藏", !(await txt("#section-about"))?.includes("Update Progress"));

// ============ 导航 toggle 语义 ============
await page.click("header button:has-text('Integrations')");
check("导航：进入集成页", await visible("#integration-view"));
await page.click("header button:has-text('Integrations')");
check("导航：再点集成回主页", await visible("#main-view"));

// ============ 控制台错误 ============
const realErrors = consoleErrors.filter((e) => !e.includes("favicon"));
check("全程无控制台错误", realErrors.length === 0, realErrors.slice(0, 3).join(" | "));

fs.writeFileSync("/tmp/tauri-smoke/results.json", JSON.stringify(results, null, 2));
const pass = results.filter((r) => r.pass).length;
console.log(`\n==== ${pass}/${results.length} 通过 ====`);
await browser.close();
devServer?.kill();
process.exit(pass === results.length ? 0 : 1);
