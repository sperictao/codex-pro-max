import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { SETTINGS_SECTIONS as SECTIONS } from "@/shared/navigation";
import { Button } from "@/shared/components/ui/button";
import { GeneralSection } from "./GeneralSection";
import { AppearanceSection } from "./AppearanceSection";
import { NetworkSection } from "./NetworkSection";
import { ModeSection } from "./ModeSection";
import { GuardSettingsSection } from "@/features/guard/GuardSettingsSection";
import { AboutSection } from "@/features/updater/AboutSection";

// 设置视图：侧栏分区（清单在 shared/navigation）+ 内容区 + 保存 footer（外观/看守/关于隐藏，旧 switchSection 行为）
export function SettingsView() {
  const { t } = useTranslation();
  const section = useAppStore((s) => s.settingsSection);
  const setSettingsSection = useAppStore((s) => s.setSettingsSection);
  const saveConfig = useAppStore((s) => s.saveConfig);
  // 保存 footer 仅在外观/看守/关于分区隐藏（旧 switchSection 行为）
  const footerHidden = section === "about" || section === "appearance" || section === "guard";

  return (
    <main className="min-h-0 flex-1" id="settings-view">
      <div className="flex h-full">
        <nav className="flex w-44 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border p-3">
          {SECTIONS.map((s) => (
            <Button
              key={s.id}
              variant={section === s.id ? "secondary" : "ghost"}
              size="sm"
              className="w-full justify-start gap-2"
              aria-current={section === s.id ? "page" : undefined}
              onClick={() => setSettingsSection(s.id)}
            >
              {s.icon}
              <span>{t(s.labelKey)}</span>
            </Button>
          ))}
        </nav>

        <div className="flex-1 overflow-y-auto p-4 md:p-6">
          {section === "general" && <GeneralSection />}
          {section === "appearance" && <AppearanceSection />}
          {section === "network" && <NetworkSection />}
          {section === "mode" && <ModeSection />}
          {section === "guard" && <GuardSettingsSection />}
          {section === "about" && <AboutSection />}
          {!footerHidden && (
            <div className="mt-4 flex justify-end border-t border-border pt-4" id="settings-footer">
              <Button id="btn-save-config" onClick={() => void saveConfig()}>
                {t("Save Settings")}
              </Button>
            </div>
          )}
        </div>
      </div>
    </main>
  );
}
