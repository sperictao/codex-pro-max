import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "../store";
import { i18n } from "../i18n";

export const REPO_URL = "https://github.com/sperictao/codex-pro-max";

// 浏览器打开本仓库（关于页 GitHub 链接与 header 软件名共用）
export async function openRepo(): Promise<void> {
  try {
    await openUrl(REPO_URL);
  } catch (e) {
    useAppStore.getState().toast(i18n.t("Failed to open link: {{error}}", { error: String(e) }), "error");
  }
}
