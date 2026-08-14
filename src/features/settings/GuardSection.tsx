// 看守设置分区：总开关（本单元）；看守文件列表与添加/编辑弹窗在单元 8 落地于本文件

import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { TOGGLE } from "@/shared/lib/ui";

export function GuardSection() {
  const { t } = useTranslation();
  const guardEnabled = useAppStore((s) => s.guardState.enabled);
  const toggleGuardEnabled = useAppStore((s) => s.toggleGuardEnabled);

  return (
    <section className="settings-section" id="section-guard">
      <h2 className="mb-4 text-base font-semibold">{t("Config Guard")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Master Switch")}</label>
        <label className="flex flex-1 cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3">
          <span className="flex flex-col gap-0.5">
            <span className="text-sm">{t("Enable Codex config guard")}</span>
            <span className="text-xs opacity-60">
              {t("Manage and lock ~/.codex config parameters; locked params are reverted automatically when changed (only while this app is running, every 60 seconds).")}
            </span>
          </span>
          <input type="checkbox" className={TOGGLE} id="settings-guard-toggle"
            checked={guardEnabled} onChange={() => void toggleGuardEnabled()} />
        </label>
      </div>
    </section>
  );
}
