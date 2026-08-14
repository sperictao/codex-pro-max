// shared/i18n：react-i18next 初始化。字典复用旧 src/i18n/ 两份文件（单元 9 再物理迁移过来）。
// key 即英文原文，缺失 key 返回 key 本身 + fallbackLng en 构成双层英文兜底（语义与旧实现及 Rust tr() 一致）。
// 语言切换由 i18next.changeLanguage 驱动，订阅组件自动重渲染——旧 rerenderDynamicText 手动清单随之消失。

import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { en } from "@/i18n/en";
import { zhCN } from "@/i18n/zh-CN";

export type ResolvedLanguage = "en" | "zh-CN";

export async function initI18n(lang: ResolvedLanguage): Promise<void> {
  await i18next.use(initReactI18next).init({
    lng: lang,
    fallbackLng: "en",
    resources: {
      en: { translation: en },
      "zh-CN": { translation: zhCN },
    },
    interpolation: { escapeValue: false },
    // key 就是原文，关掉 key 分隔/命名空间分隔语义
    nsSeparator: false,
    keySeparator: false,
  });
}

export function currentLanguage(): ResolvedLanguage {
  return i18next.language === "zh-CN" ? "zh-CN" : "en";
}

// 非组件上下文（store action 等）直接取 t；组件内请用 useTranslation
export const i18n = i18next;
