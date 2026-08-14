// 模式分区：启动模式与自动打开浏览器。标签文案随开关态派生（旧 updateModeLabel/updateAutoOpenLabel）

import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { TOGGLE } from "@/shared/lib/ui";

export function ModeSection() {
  const { t } = useTranslation();
  const config = useAppStore((s) => s.config);
  const setConfigField = useAppStore((s) => s.setConfigField);
  const separateWindow = config?.separate_window_mode ?? false;
  const autoOpen = config?.auto_open ?? true;

  return (
    <section className="settings-section" id="section-mode">
      <h2 className="mb-4 text-base font-semibold">{t("Mode")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Launch Mode")}</label>
        <label className="flex flex-1 cursor-pointer items-center gap-3">
          <input type="checkbox" className={TOGGLE} id="toggle-mode" checked={separateWindow}
            onChange={(e) => setConfigField({ separate_window_mode: e.target.checked })} />
          <span className="text-sm">
            {separateWindow
              ? t("Separate window mode (does not restart Codex)")
              : t("Full launch mode (restarts Codex)")}
          </span>
        </label>
      </div>

      <div className="flex items-start gap-4 py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Auto-open Browser")}</label>
        <label className="flex flex-1 cursor-pointer items-center gap-3">
          <input type="checkbox" className={TOGGLE} id="toggle-auto-open" checked={autoOpen}
            onChange={(e) => setConfigField({ auto_open: e.target.checked })} />
          <span className="text-sm">
            {autoOpen ? t("Open browser automatically on start") : t("Do not open browser automatically")}
          </span>
        </label>
      </div>
    </section>
  );
}
