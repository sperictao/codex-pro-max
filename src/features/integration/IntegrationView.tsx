// 集成视图：FastCtx 卡片（dsh 卡片已随 e7bc945 迁出本仓）
// 卡片挂载时检测一次（旧行为：点击导航进入时刷新状态）

import { useTranslation } from "react-i18next";
import { FastctxCard } from "./FastctxCard";

export function IntegrationView() {
  const { t } = useTranslation();
  return (
    <main className="flex-1 overflow-y-auto p-4 md:p-6" id="integration-view">
      <h2 className="mb-4 text-base font-semibold">{t("Integrations")}</h2>
      <FastctxCard />
    </main>
  );
}
