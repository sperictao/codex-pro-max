// 通用分区：语言、三个路径（浏览/校验）、系统行为（托盘/自启）、日志入口
// 路径校验为响应式 effect（路径字段变化即重跑），替代旧 onConfigChange→validatePaths 手动触发；
// 校验结果存 kind+key/raw，渲染时过 t()——切语言无需重新 invoke（旧实现靠重跑 validatePaths 达到同效）

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { useAppStore } from "@/shared/store";
import * as cmd from "@/shared/commands";
import { SelectCard } from "@/shared/components/SelectCard";
import { BTN, INPUT_MONO, TOGGLE } from "@/shared/lib/ui";

const LANG_OPTIONS = [
  { id: "system", labelKey: "Follow System" },
  { id: "en", labelKey: "English" },
  { id: "zh-CN", labelKey: "中文" },
] as const;

type CheckResult = { cls: "" | "ok" | "err"; textKey: string | null; raw: string | null };
const EMPTY: CheckResult = { cls: "", textKey: null, raw: null };

function CheckSpan({ result }: { result: CheckResult }) {
  const { t } = useTranslation();
  return (
    <span className={`config-validate${result.cls ? ` ${result.cls}` : ""}`}>
      {result.raw ?? (result.textKey ? t(result.textKey) : "")}
    </span>
  );
}

export function GeneralSection() {
  const { t } = useTranslation();
  const config = useAppStore((s) => s.config);
  const autostart = useAppStore((s) => s.autostart);
  const languageSetting = useAppStore((s) => s.languageSetting);
  const setLanguageSetting = useAppStore((s) => s.setLanguageSetting);
  const setConfigField = useAppStore((s) => s.setConfigField);
  const toggleAutostart = useAppStore((s) => s.toggleAutostart);
  const toast = useAppStore((s) => s.toast);

  const [pathCheck, setPathCheck] = useState<CheckResult>(EMPTY);
  const [nodeCheck, setNodeCheck] = useState<CheckResult>(EMPTY);
  const [codexCheck, setCodexCheck] = useState<CheckResult>(EMPTY);
  const seqRef = useRef(0);

  const taskboardPath = config?.taskboard_path ?? "";
  const nodePath = config?.node_path ?? "";
  const codexPath = config?.codex_app_path ?? "";

  useEffect(() => {
    const seq = ++seqRef.current;
    void (async () => {
      // taskboard 路径：空则不显示
      let path: CheckResult = EMPTY;
      if (taskboardPath) {
        try {
          path = (await cmd.validateTaskboardPath(taskboardPath))
            ? { cls: "ok", textKey: "Valid", raw: null }
            : { cls: "err", textKey: "Invalid", raw: null };
        } catch {
          path = { cls: "err", textKey: "Check failed", raw: null };
        }
      }
      // node：空路径也检查（用系统 PATH 的 node）
      let node: CheckResult;
      try {
        node = { cls: "ok", textKey: null, raw: await cmd.checkNodeVersion(nodePath) };
      } catch {
        node = { cls: "err", textKey: "Unavailable", raw: null };
      }
      // codex app：空则不显示
      let codex: CheckResult = EMPTY;
      if (codexPath) {
        try {
          codex = (await cmd.checkCodexApp(codexPath))
            ? { cls: "ok", textKey: "Exists", raw: null }
            : { cls: "err", textKey: "Not found", raw: null };
        } catch {
          codex = { cls: "err", textKey: "Check failed", raw: null };
        }
      }
      if (seq !== seqRef.current) return;
      setPathCheck(path);
      setNodeCheck(node);
      setCodexCheck(codex);
    })();
  }, [taskboardPath, nodePath, codexPath]);

  const browsePath = async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (selected) setConfigField({ taskboard_path: selected as string });
  };
  const browseNode = async () => {
    const selected = await openDialog({ directory: false, multiple: false, filters: [{ name: "Node", extensions: ["*"] }] });
    if (selected) setConfigField({ node_path: selected as string });
  };
  const browseCodex = async () => {
    // macOS 选 .app 包（目录）；Windows 选 .exe 文件，目录选择器选不到 exe
    const isWindows = navigator.userAgent.includes("Windows");
    const selected = await openDialog(
      isWindows
        ? { directory: false, multiple: false, filters: [{ name: "Codex", extensions: ["exe"] }] }
        : { directory: true, multiple: false },
    );
    if (selected) setConfigField({ codex_app_path: selected as string });
  };
  const useBundled = async () => {
    try {
      const path = await cmd.getBundledTaskboardPath();
      if (path) {
        setConfigField({ taskboard_path: path });
        toast(t("Using bundled Taskboard path"), "success");
      } else {
        toast(t("Bundled Taskboard not found"), "error");
      }
    } catch (e) {
      toast(t("Failed to get bundled path: {{error}}", { error: String(e) }), "error");
    }
  };
  const openLogDir = async () => {
    try {
      await openUrl(await cmd.getLogDir());
    } catch (e) {
      toast(String(e), "error");
    }
  };

  return (
    <section className="settings-section" id="section-general">
      <h2 className="mb-4 text-base font-semibold">{t("General")}</h2>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Language")}</label>
        <div className="flex flex-1 gap-3">
          {LANG_OPTIONS.map((opt) => (
            <SelectCard key={opt.id} selected={languageSetting === opt.id} onClick={() => void setLanguageSetting(opt.id)}>
              <span className="text-sm">{t(opt.labelKey)}</span>
            </SelectCard>
          ))}
        </div>
      </div>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-2 text-sm font-medium" htmlFor="cfg-path">{t("Taskboard Path")}</label>
        <div className="flex flex-1 items-center gap-2">
          <input type="text" className={INPUT_MONO} id="cfg-path" placeholder="/path/to/dashi-taskboard"
            value={taskboardPath} onChange={(e) => setConfigField({ taskboard_path: e.target.value })} />
          <button className={BTN} onClick={() => void browsePath()}>{t("Browse")}</button>
          <button className={BTN} onClick={() => void useBundled()}>{t("Use Bundled")}</button>
          <CheckSpan result={pathCheck} />
        </div>
      </div>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-2 text-sm font-medium" htmlFor="cfg-node">{t("Node.js Path")}</label>
        <div className="flex flex-1 items-center gap-2">
          <input type="text" className={INPUT_MONO} id="cfg-node" placeholder={t("Leave empty to use node from PATH")}
            value={nodePath} onChange={(e) => setConfigField({ node_path: e.target.value })} />
          <button className={BTN} onClick={() => void browseNode()}>{t("Browse")}</button>
          <CheckSpan result={nodeCheck} />
        </div>
      </div>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-2 text-sm font-medium" htmlFor="cfg-codex">{t("Codex App Path")}</label>
        <div className="flex flex-1 items-center gap-2">
          <input type="text" className={INPUT_MONO} id="cfg-codex" placeholder="/Applications/ChatGPT.app"
            value={codexPath} onChange={(e) => setConfigField({ codex_app_path: e.target.value })} />
          <button className={BTN} onClick={() => void browseCodex()}>{t("Browse")}</button>
          <CheckSpan result={codexCheck} />
        </div>
      </div>

      <div className="flex items-start gap-4 border-b border-border py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("System Behavior")}</label>
        <div className="flex flex-1 flex-col gap-2">
          <label className="flex cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3">
            <span className="flex flex-col gap-0.5">
              <span className="text-sm">{t("Minimize to tray when closing window")}</span>
              <span className="text-xs opacity-60">
                {t("When enabled, the close button hides the window and the app keeps running in the system tray.")}
              </span>
            </span>
            <input type="checkbox" className={TOGGLE} id="toggle-tray"
              checked={config?.minimize_to_tray_on_close ?? false}
              onChange={(e) => setConfigField({ minimize_to_tray_on_close: e.target.checked })} />
          </label>
          <label className="flex cursor-pointer items-center justify-between gap-4 rounded-lg border border-border p-3">
            <span className="flex flex-col gap-0.5">
              <span className="text-sm">{t("Launch at login")}</span>
              <span className="text-xs opacity-60">
                {t("When enabled, the app starts silently in the system tray when you log in.")}
              </span>
            </span>
            <input type="checkbox" className={TOGGLE} id="toggle-autostart"
              checked={autostart} onChange={() => void toggleAutostart()} />
          </label>
        </div>
      </div>

      <div className="flex items-start gap-4 py-4">
        <label className="w-36 shrink-0 pt-1 text-sm font-medium">{t("Logs")}</label>
        <div className="flex flex-1 items-center justify-between gap-4 rounded-lg border border-border p-3">
          <span className="flex flex-col gap-0.5">
            <span className="text-sm">{t("Open log folder")}</span>
            <span className="text-xs opacity-60">
              {t("Logs are written to files only; open the folder when something goes wrong.")}
            </span>
          </span>
          <button className={BTN} onClick={() => void openLogDir()}>{t("Open")}</button>
        </div>
      </div>
    </section>
  );
}
