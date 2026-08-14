import { DEFAULT_FAMILY, THEME_FAMILIES } from "./theme-families";

export type ThemeMode = "light" | "dark" | "system";

// 族清单的唯一事实来源是生成的 manifest（ADR 0008）；
// 亮暗配对坍缩为命名约定 <族id>-light|dark，不再维护配对表
const FAMILY_IDS: ReadonlySet<string> = new Set(THEME_FAMILIES.map((f) => f.id));

export function getStoredTheme(stored: string | null): ThemeMode {
  if (stored === "light" || stored === "dark" || stored === "system") {
    return stored;
  }
  return "system";
}

export function getStoredFamily(stored: string | null): string {
  return stored !== null && FAMILY_IDS.has(stored) ? stored : DEFAULT_FAMILY;
}

export function resolveDataTheme(mode: ThemeMode, family: string, prefersDark: boolean): string {
  const id = FAMILY_IDS.has(family) ? family : DEFAULT_FAMILY;
  const dark = mode === "dark" || (mode === "system" && prefersDark);
  return `${id}-${dark ? "dark" : "light"}`;
}
