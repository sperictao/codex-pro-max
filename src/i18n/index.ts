import i18next from "i18next";
import { en } from "./en";
import { zhCN } from "./zh-CN";

export type ResolvedLanguage = "en" | "zh-CN";

/**
 * 初始化 i18next（资源内联，init 同步完成）。
 * key 即英文原文，缺失 key 时 i18next 默认返回 key 本身——
 * 与 fallbackLng: "en" 叠加构成双层英文兜底，和 Rust 侧 tr() 语义一致。
 */
export function initI18n(lang: ResolvedLanguage): void {
  i18next.init({
    lng: lang,
    fallbackLng: "en",
    resources: {
      en: { translation: en },
      "zh-CN": { translation: zhCN },
    },
    interpolation: { escapeValue: false },
    // key 就是原文，关掉 key 分隔/命名空间分隔语义（复数/语境后缀我们的 key 用不到）
    nsSeparator: false,
    keySeparator: false,
  });
}

export function currentLanguage(): ResolvedLanguage {
  return i18next.language === "zh-CN" ? "zh-CN" : "en";
}

/** 翻译单条字符串；params 用 i18next 插值（{{name}}） */
export function t(key: string, params?: Record<string, string | number>): string {
  return i18next.t(key, params);
}

/**
 * 扫描并翻译 DOM 中的静态文本挂载点：
 * [data-i18n] → textContent，[data-i18n-placeholder] → placeholder，
 * [data-i18n-title] → title，[data-i18n-aria] → aria-label
 */
export function applyDomTranslations(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    el.textContent = t(el.getAttribute("data-i18n")!);
  });
  root.querySelectorAll<HTMLInputElement>("[data-i18n-placeholder]").forEach((el) => {
    el.placeholder = t(el.getAttribute("data-i18n-placeholder")!);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    el.title = t(el.getAttribute("data-i18n-title")!);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-aria]").forEach((el) => {
    el.setAttribute("aria-label", t(el.getAttribute("data-i18n-aria")!));
  });
}

/** 切换语言并立即重刷 DOM 静态文本（动态内容由调用处重渲染） */
export async function applyLanguage(lang: ResolvedLanguage): Promise<void> {
  await i18next.changeLanguage(lang);
  document.documentElement.lang = lang;
  applyDomTranslations();
}
