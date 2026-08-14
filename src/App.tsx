import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { getVersion } from "@tauri-apps/api/app";
import { useAppStore, type View } from "./shared/store";
import { onDshStep, onStatusUpdate, onUpdaterDownloadProgress } from "./shared/events";
import * as cmd from "./shared/commands";
import { currentConfigDraft } from "./shared/config";
import { i18n } from "./shared/i18n";
import { Toaster } from "./shared/components/Toaster";
import { openRepo } from "./shared/lib/links";
import { UpdateBadge } from "./features/updater/UpdateBadge";
import { HomeView } from "./features/home/HomeView";
import { SettingsView } from "./features/settings/SettingsView";
import { SkillView } from "./features/skill/SkillView";
import { GuardView } from "./features/guard/GuardView";
import { IntegrationView } from "./features/integration/IntegrationView";

// Skill 按钮在旧 UI 无 data-i18n（恒为英文），其余走词典
const NAV_ITEMS: { view: View; labelKey: string | null }[] = [
  { view: "home", labelKey: "Home" },
  { view: "skill", labelKey: null },
  { view: "guard", labelKey: "Guard" },
  { view: "integration", labelKey: "Integrations" },
  { view: "settings", labelKey: "Settings" },
];

export function App() {
  const { t } = useTranslation();
  const activeView = useAppStore((s) => s.activeView);
  const guardEnabled = useAppStore((s) => s.guardState.enabled);
  const navigate = useAppStore((s) => s.navigate);

  // 事件桥 + 初始化 + 3s 状态轮询
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const bind = (p: Promise<() => void>) => {
      void p.then((u) => {
        if (disposed) u();
        else unlisteners.push(u);
      });
    };
    bind(
      onStatusUpdate((p) =>
        useAppStore.getState().updateService({ name: p.name, status: p.status, pid: null, message: p.message }),
      ),
    );
    bind(onUpdaterDownloadProgress((p) => useAppStore.getState().setDownloadProgress(p)));
    bind(onDshStep((s) => useAppStore.getState().handleDshStep(s)));

    // 进程事故通知需要系统授权（macOS），启动时静默请求一次
    void (async () => {
      try {
        if (!(await isPermissionGranted())) await requestPermission();
      } catch {
        /* 拒绝则通知静默失败，不打扰 */
      }
    })();

    void (async () => {
      try {
        const cfg = await cmd.loadConfig();
        if (disposed) return;
        useAppStore.getState().applyConfig(cfg);
        try {
          const autostart = await cmd.autostartIsEnabled();
          if (!disposed) useAppStore.getState().setAutostart(autostart);
        } catch {
          /* 读不到就当关 */
        }

        // 应用版本（关于页）
        try {
          useAppStore.getState().setAppVersion(await getVersion());
        } catch {
          useAppStore.getState().setAppVersion("unknown");
        }

        // codex 路径为空或已失效时，自动探测真实安装位置并回填落盘
        const codexPath = useAppStore.getState().config?.codex_app_path ?? "";
        const currentValid = codexPath !== "" && (await cmd.checkCodexApp(codexPath));
        if (!currentValid) {
          const found = await cmd.detectCodexApp();
          if (found && !disposed) {
            useAppStore.getState().setConfigField({ codex_app_path: found });
            await cmd.updateSettings(currentConfigDraft(useAppStore.getState()));
          }
        }

        // 更新源健康检查 + 静默检查更新（有新版本才提示）
        await useAppStore.getState().refreshUpdaterHealth();
        void useAppStore.getState().checkForUpdates(true);
      } catch (e) {
        useAppStore.getState().toast(i18n.t("Initialization failed: {{error}}", { error: String(e) }), "error");
      }
    })();

    void useAppStore.getState().refreshStatus();
    // 状态 + 看守视图轮询（每 3 秒；看守视图不在前台时 refreshGuardView 自身跳过）
    const timer = setInterval(() => {
      void useAppStore.getState().refreshStatus();
      void useAppStore.getState().refreshGuardView();
    }, 3000);

    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
      clearInterval(timer);
    };
  }, []);

  // 跟随系统模式：OS 亮暗切换时重解析 data-theme
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => useAppStore.getState().syncSystemTheme();
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  // 看守总开关关闭且当前在看守页 → 跳回主页（旧 renderGuardToggle 行为）
  useEffect(() => {
    if (!guardEnabled && activeView === "guard") navigate("home");
  }, [guardEnabled, activeView, navigate]);

  return (
    <>
      <header className="flex shrink-0 items-center justify-between border-b border-border px-4 py-2.5">
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="cursor-pointer text-sm font-semibold"
            title="GitHub"
            onClick={() => void openRepo()}
          >
            Codex Pro Max
          </button>
          <UpdateBadge />
        </div>
        <div className="flex items-center gap-1">
          {NAV_ITEMS.filter((item) => item.view !== "guard" || guardEnabled).map((item) => (
            <button
              key={item.view}
              className={`header-btn${activeView === item.view ? " active" : ""}`}
              onClick={() => navigate(item.view)}
            >
              {item.labelKey ? t(item.labelKey) : "Skill"}
            </button>
          ))}
        </div>
      </header>

      {activeView === "home" && <HomeView />}
      {activeView === "settings" && <SettingsView />}
      {activeView === "skill" && <SkillView />}
      {activeView === "guard" && <GuardView />}
      {activeView === "integration" && <IntegrationView />}
      <Toaster />
    </>
  );
}
