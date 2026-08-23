// 集成视图：dsh 卡片置顶 + FastCtx 卡片（旧 index.html 顺序）
// 两张卡片挂载时各自检测一次（旧行为：点击导航进入时刷新两者状态）

import { useTranslation } from "react-i18next";
import { FastctxCard } from "./FastctxCard";

export function IntegrationView() {
  const { t } = useTranslation();
  return (
    <main className="flex-1 overflow-y-auto p-6" id="integration-view">
      <h2 className="mb-4 text-base font-semibold">{t("Integrations")}</h2>
      <FastctxCard />
    </main>
  );
}
