import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { getVersion } from "@tauri-apps/api/app";
import { useAppStore } from "./shared/store";
import { onStatusUpdate, onUpdaterDownloadProgress } from "./shared/events";
import * as cmd from "./shared/commands";
import { log } from "./shared/logger";
import { currentConfigDraft } from "./shared/config";
import { i18n } from "./shared/i18n";
import { Toaster } from "./shared/components/Toaster";
import { CommandPalette } from "./shared/components/CommandPalette";
import { NAV_ITEMS } from "./shared/navigation";
import { Button } from "./shared/components/ui/button";
import { openRepo } from "./shared/lib/links";
import { UpdateBadge } from "./features/updater/UpdateBadge";
import { SearchIcon } from "lucide-react";
import { HomeView } from "./features/home/HomeView";
import { SettingsView } from "./features/settings/SettingsView";
import { SkillView } from "./features/skill/SkillView";
import { GuardView } from "./features/guard/GuardView";
import { IntegrationView } from "./features/integration/IntegrationView";
import { ModelView } from "./features/models/ModelView";

// 快捷键提示按平台显示（macOS ⌘ / 其余 Ctrl+）
const PRIMARY_MODIFIER =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";

export function App() {
  const { t } = useTranslation();
  const activeView = useAppStore((s) => s.activeView);
  const guardEnabled = useAppStore((s) => s.guardState.enabled);
  const goto = useAppStore((s) => s.goto);
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Cmd/Ctrl+K 开关命令面板
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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

    // 进程事故通知需要系统授权（macOS），启动时静默请求一次
    void (async () => {
      try {
        if (!(await isPermissionGranted())) await requestPermission();
      } catch (e) {
        log.warn("请求通知权限失败（静默）", e);
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
        } catch (e) {
          log.warn("读取应用版本失败", e);
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
        toast.error(i18n.t("Initialization failed: {{error}}", { error: String(e) }));
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
    if (!guardEnabled && activeView === "guard") goto("home");
  }, [guardEnabled, activeView, goto]);

  return (
    <>
      <header className="flex h-(--header-height) shrink-0 items-center justify-between border-b border-border px-4">
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" className="font-semibold" title="GitHub" onClick={() => void openRepo()}>
            Codex Pro Max
          </Button>
          <UpdateBadge />
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="gap-2 text-muted-foreground"
            aria-label={t("Command Palette")}
            onClick={() => setPaletteOpen(true)}
          >
            <SearchIcon className="size-4" />
            <kbd className="pointer-events-none hidden h-5 items-center rounded border border-border bg-muted px-1.5 font-mono text-[10px] font-medium sm:inline-flex">
              {PRIMARY_MODIFIER}K
            </kbd>
          </Button>
          {NAV_ITEMS.filter((item) => item.view !== "guard" || guardEnabled).map((item) => (
            <Button
              key={item.view}
              variant={activeView === item.view ? "secondary" : "ghost"}
              size="sm"
              aria-current={activeView === item.view ? "page" : undefined}
              onClick={() => {
                const toggleBack =
                  activeView === item.view && (item.view === "settings" || item.view === "integration");
                goto(toggleBack ? "home" : item.view);
              }}
            >
              {item.labelKey ? t(item.labelKey) : "Skill"}
            </Button>
          ))}
        </div>
      </header>

      {activeView === "home" && <HomeView />}
      {activeView === "settings" && <SettingsView />}
      {activeView === "skill" && <SkillView />}
      {activeView === "guard" && <GuardView />}
      {activeView === "models" && <ModelView />}
      {activeView === "integration" && <IntegrationView />}
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
      <Toaster />
    </>
  );
}
