import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";
import { SelectCard } from "@/shared/components/SelectCard";

// 通用分区（单元 2 先落地语言行；路径/系统行为/日志行在单元 4 补齐）
const LANG_OPTIONS = [
  { id: "system", labelKey: "Follow System" },
  { id: "en", labelKey: "English" },
  { id: "zh-CN", labelKey: "中文" },
] as const;

export function GeneralSection() {
  const { t } = useTranslation();
  const languageSetting = useAppStore((s) => s.languageSetting);
  const setLanguageSetting = useAppStore((s) => s.setLanguageSetting);

  return (
    <section className="settings-section" id="section-general">
      <h2 className="mb-4 text-base font-semibold">{t("General")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Language")}</label>
        <div className="flex flex-1 gap-3">
          {LANG_OPTIONS.map((opt) => (
            <SelectCard
              key={opt.id}
              selected={languageSetting === opt.id}
              onClick={() => void setLanguageSetting(opt.id)}
            >
              <span className="text-sm">{t(opt.labelKey)}</span>
            </SelectCard>
          ))}
        </div>
      </div>
    </section>
  );
}
