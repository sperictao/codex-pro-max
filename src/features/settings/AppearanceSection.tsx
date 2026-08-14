import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { SelectCard } from "@/shared/components/SelectCard";
import { THEME_FAMILIES } from "@/theme-families";
import type { ThemeMode } from "@/theme";

// 外观分区：模式三卡 + 41 族色板卡网格（色板卡局部 data-theme 渲染该族亮主题缩略）
const MODE_OPTIONS: { id: ThemeMode; labelKey: string; preview: "both" | "light" | "dark" }[] = [
  { id: "system", labelKey: "Follow System", preview: "both" },
  { id: "light", labelKey: "Light", preview: "light" },
  { id: "dark", labelKey: "Dark", preview: "dark" },
];

function ModePreview({ panes }: { panes: ("light" | "dark")[] }) {
  return (
    <span className="mode-preview">
      {panes.map((p) => (
        <span key={p} className={`mp-pane mp-${p}`}>
          <span className="mp-title"></span>
          <span className="mp-line"></span>
          <span className="mp-line mp-short"></span>
          <span className="mp-btn"></span>
        </span>
      ))}
    </span>
  );
}

export function AppearanceSection() {
  const { t } = useTranslation();
  const themeMode = useAppStore((s) => s.themeMode);
  const themeFamily = useAppStore((s) => s.themeFamily);
  const setThemeMode = useAppStore((s) => s.setThemeMode);
  const setThemeFamily = useAppStore((s) => s.setThemeFamily);

  return (
    <section className="settings-section" id="section-appearance">
      <h2 className="mb-4 text-base font-semibold">{t("Appearance")}</h2>

      <div className="mb-2 text-sm font-medium">{t("Mode")}</div>
      <div className="mb-6 flex gap-3">
        {MODE_OPTIONS.map((opt) => (
          <SelectCard key={opt.id} selected={themeMode === opt.id} onClick={() => setThemeMode(opt.id)}>
            <ModePreview panes={opt.preview === "both" ? ["light", "dark"] : [opt.preview]} />
            <span className="text-sm">{t(opt.labelKey)}</span>
          </SelectCard>
        ))}
      </div>

      <div className="mb-2 text-sm font-medium">{t("Theme")}</div>
      <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 lg:grid-cols-6" id="theme-family-grid">
        {THEME_FAMILIES.map((f) => (
          <SelectCard key={f.id} selected={themeFamily === f.id} onClick={() => setThemeFamily(f.id)}>
            <span className="family-preview" data-theme={`${f.id}-light`}>
              <span className="fp-dots">
                <span className="fp-dot bg-primary"></span>
                <span className="fp-dot bg-secondary"></span>
                <span className="fp-dot bg-accent"></span>
                <span className="fp-dot bg-muted"></span>
              </span>
              <span className="fp-bar w-full"></span>
              <span className="fp-bar w-2/3"></span>
            </span>
            <span className="text-xs">{f.label}</span>
          </SelectCard>
        ))}
      </div>

      <p className="mt-4 text-xs opacity-60">
        {t("Theme changes apply immediately. With Follow System, the theme switches automatically with the OS appearance.")}
      </p>
    </section>
  );
}
