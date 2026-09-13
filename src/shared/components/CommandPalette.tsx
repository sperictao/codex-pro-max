import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { ExternalLinkIcon, FolderOpenIcon, MoonIcon, RefreshCwIcon, SunIcon } from "lucide-react";
import { open as openPath } from "@tauri-apps/plugin-shell";
import { toast } from "sonner";
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/shared/components/ui/command";
import { NAV_ITEMS, SETTINGS_SECTIONS } from "@/shared/navigation";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { openRepo } from "@/shared/lib/links";

// 命令面板（Cmd/Ctrl+K）：导航与全局动作的第二入口，不替代顶栏导航。
// 命令清单复用 NAV_ITEMS / SETTINGS_SECTIONS，写死枚举，不做注册与历史。
export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (next: boolean) => void }) {
  const { t } = useTranslation();
  const activeView = useAppStore((s) => s.activeView);
  const section = useAppStore((s) => s.settingsSection);
  const guardEnabled = useAppStore((s) => s.guardState.enabled);
  const themeMode = useAppStore((s) => s.themeMode);
  const goto = useAppStore((s) => s.goto);
  const setThemeMode = useAppStore((s) => s.setThemeMode);
  const checkForUpdates = useAppStore((s) => s.checkForUpdates);

  const run = useCallback(
    (action: () => void) => {
      onOpenChange(false);
      action();
    },
    [onOpenChange],
  );

  const openLogDir = useCallback(async () => {
    try {
      await openPath(await cmd.getLogDir());
    } catch (e) {
      toast.error(String(e));
    }
  }, []);

  const dark = themeMode === "dark";

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("Command Palette")}
      description={t("Search commands…")}
    >
      <Command>
          <CommandInput placeholder={t("Search commands…")} />
        <CommandList>
          <CommandEmpty>{t("No results.")}</CommandEmpty>

          <CommandGroup heading={t("Navigation")}>
            {NAV_ITEMS.filter((item) => item.view !== "guard" || guardEnabled).map((item) => {
              const label = item.labelKey ? t(item.labelKey) : "Skill";
              return (
                <CommandItem
                  key={item.view}
                  value={label}
                  data-checked={activeView === item.view}
                  onSelect={() => run(() => goto(item.view))}
                >
                  {label}
                </CommandItem>
              );
            })}
          </CommandGroup>

          <CommandSeparator />

          <CommandGroup heading={t("Settings")}>
            {SETTINGS_SECTIONS.map((s) => (
              <CommandItem
                key={s.id}
                value={t(s.labelKey)}
                data-checked={activeView === "settings" && section === s.id}
                onSelect={() => run(() => goto("settings", s.id))}
              >
                {s.icon}
                {t(s.labelKey)}
              </CommandItem>
            ))}
          </CommandGroup>

          <CommandSeparator />

          <CommandGroup heading={t("Actions")}>
            <CommandItem value={t("Check for Updates")} onSelect={() => run(() => void checkForUpdates(false))}>
              <RefreshCwIcon />
              {t("Check for Updates")}
            </CommandItem>
            <CommandItem value={t("Open GitHub Repository")} onSelect={() => run(() => void openRepo())}>
              <ExternalLinkIcon />
              {t("Open GitHub Repository")}
            </CommandItem>
            <CommandItem value={t("Open Log Directory")} onSelect={() => run(() => void openLogDir())}>
              <FolderOpenIcon />
              {t("Open Log Directory")}
            </CommandItem>
            <CommandItem
              value={dark ? t("Switch to Light Mode") : t("Switch to Dark Mode")}
              onSelect={() => run(() => setThemeMode(dark ? "light" : "dark"))}
            >
              {dark ? <SunIcon /> : <MoonIcon />}
              {dark ? t("Switch to Light Mode") : t("Switch to Dark Mode")}
            </CommandItem>
          </CommandGroup>
          </CommandList>
      </Command>
    </CommandDialog>
  );
}
