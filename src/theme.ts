export type ThemeMode = "light" | "dark" | "system";

export interface ThemeFamily {
  light: string;
  dark?: string;
  label: string;
}

export const THEME_FAMILIES: Readonly<Record<string, ThemeFamily>> = {
  geist: { light: "geist-light", dark: "geist-dark", label: "Geist" },
  light: { light: "light", dark: "dark", label: "Light" },
  cupcake: { light: "cupcake", dark: "forest", label: "Cupcake" },
  bumblebee: { light: "bumblebee", dark: "coffee", label: "Bumblebee" },
  emerald: { light: "emerald", dark: "forest", label: "Emerald" },
  corporate: { light: "corporate", dark: "business", label: "Corporate" },
  retro: { light: "retro", dark: "synthwave", label: "Retro" },
  cyberpunk: { light: "cyberpunk", dark: "synthwave", label: "Cyberpunk" },
  valentine: { light: "valentine", dark: "dracula", label: "Valentine" },
  garden: { light: "garden", dark: "forest", label: "Garden" },
  lofi: { light: "lofi", dark: "black", label: "Lo-Fi" },
  pastel: { light: "pastel", dark: "dracula", label: "Pastel" },
  fantasy: { light: "fantasy", dark: "forest", label: "Fantasy" },
  wireframe: { light: "wireframe", dark: "black", label: "Wireframe" },
  cmyk: { light: "cmyk", dark: "dark", label: "CMYK" },
  autumn: { light: "autumn", dark: "coffee", label: "Autumn" },
  acid: { light: "acid", dark: "night", label: "Acid" },
  lemonade: { light: "lemonade", dark: "dark", label: "Lemonade" },
  nord: { light: "nord", dark: "dim", label: "Nord" },
  winter: { light: "winter", dark: "night", label: "Winter" },
  caramellatte: { light: "caramellatte", dark: "coffee", label: "Caramel Latte" },
  silk: { light: "silk", dark: "abyss", label: "Silk" },
};

function hasFamily(families: Readonly<Record<string, ThemeFamily>>, family: string): boolean {
  return Object.prototype.hasOwnProperty.call(families, family);
}

export function getStoredTheme(stored: string | null): ThemeMode {
  if (stored === "light" || stored === "dark" || stored === "system") {
    return stored;
  }
  return "system";
}

export function getStoredFamily(
  stored: string | null,
  families: Readonly<Record<string, ThemeFamily>> = THEME_FAMILIES,
): string {
  return stored && hasFamily(families, stored) ? stored : "geist";
}

export function resolveDataTheme(
  mode: ThemeMode,
  family: string,
  prefersDark: boolean,
  families: Readonly<Record<string, ThemeFamily>> = THEME_FAMILIES,
): string {
  const familyId = hasFamily(families, family) ? family : "geist";
  const selected = families[familyId] ?? THEME_FAMILIES.geist;
  if (mode === "light") return selected.light;
  if (mode === "dark" || prefersDark) return selected.dark ?? "dark";
  return selected.light;
}
