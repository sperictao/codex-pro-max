// 关于分区：应用版本（本单元）；更新源健康/检查更新/进度/GitHub 链接在单元 7 落地于本文件

import { useTranslation } from "react-i18next";
import { useAppStore } from "@/shared/store";

export function AboutSection() {
  const { t } = useTranslation();
  const appVersion = useAppStore((s) => s.appVersion);

  return (
    <section className="settings-section" id="section-about">
      <h2 className="mb-4 text-base font-semibold">{t("About")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-3">
        <span className="w-36 shrink-0 text-sm font-medium">{t("App Version")}</span>
        <span className="font-mono text-sm" id="about-version">{appVersion}</span>
      </div>
    </section>
  );
}
