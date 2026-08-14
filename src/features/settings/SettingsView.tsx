import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore, type SettingsSection } from "@/shared/store";
import { BTN_PRIMARY } from "@/shared/lib/ui";
import { GeneralSection } from "./GeneralSection";
import { AppearanceSection } from "./AppearanceSection";
import { NetworkSection } from "./NetworkSection";
import { ModeSection } from "./ModeSection";
import { GuardSettingsSection } from "@/features/guard/GuardSettingsSection";
import { AboutSection } from "@/features/updater/AboutSection";

// 设置视图：侧栏 6 分区 + 内容区 + 保存 footer（外观/看守/关于隐藏，旧 switchSection 行为）
const SECTIONS: { id: SettingsSection; labelKey: string; icon: ReactNode }[] = [
  {
    id: "general",
    labelKey: "General",
    icon: (
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="4" y1="21" x2="4" y2="14" /><line x1="4" y1="10" x2="4" y2="3" /><line x1="12" y1="21" x2="12" y2="12" /><line x1="12" y1="8" x2="12" y2="3" /><line x1="20" y1="21" x2="20" y2="16" /><line x1="20" y1="12" x2="20" y2="3" /><line x1="1" y1="14" x2="7" y2="14" /><line x1="9" y1="8" x2="15" y2="8" /><line x1="17" y1="16" x2="23" y2="16" /></svg>
    ),
  },
  {
    id: "appearance",
    labelKey: "Appearance",
    icon: (
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="13.5" cy="6.5" r=".5" /><circle cx="17.5" cy="10.5" r=".5" /><circle cx="8.5" cy="7.5" r=".5" /><circle cx="6.5" cy="12.5" r=".5" /><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z" /></svg>
    ),
  },
  {
    id: "network",
    labelKey: "Network",
    icon: (
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10" /><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" /><path d="M2 12h20" /></svg>
    ),
  },
  {
    id: "mode",
    labelKey: "Mode",
    icon: (
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z" /><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65" /><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65" /></svg>
    ),
  },
  {
    id: "guard",
    labelKey: "Guard",
    icon: (
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10" /></svg>
    ),
  },
  {
    id: "about",
    labelKey: "About",
    icon: (
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10" /><path d="M12 16v-4" /><path d="M12 8h.01" /></svg>
    ),
  },
];

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
            <button
              key={s.id}
              className={`nav-item${section === s.id ? " active" : ""}`}
              onClick={() => setSettingsSection(s.id)}
            >
              {s.icon}
              <span>{t(s.labelKey)}</span>
            </button>
          ))}
        </nav>

        <div className="flex-1 overflow-y-auto p-6">
          {section === "general" && <GeneralSection />}
          {section === "appearance" && <AppearanceSection />}
          {section === "network" && <NetworkSection />}
          {section === "mode" && <ModeSection />}
          {section === "guard" && <GuardSettingsSection />}
          {section === "about" && <AboutSection />}
          {!footerHidden && (
            <div className="mt-4 flex justify-end border-t border-border pt-4" id="settings-footer">
              <button className={BTN_PRIMARY} id="btn-save-config" onClick={() => void saveConfig()}>
                {t("Save Settings")}
              </button>
            </div>
          )}
        </div>
      </div>
    </main>
  );
}
