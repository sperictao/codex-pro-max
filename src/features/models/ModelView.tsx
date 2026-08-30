// 模型配置视图：~/.codex/config.toml 模型域的可视化管理（参考 CCursor 的 Model Config）。
// config.toml 是唯一事实来源：当前模型三键（空 = 回落默认）与 [model_providers.*] 直接读写；
// 预设库（快速切换的组合）存启动器配置。无锁定语义，不进 3s 轮询：
// 进入视图与每次操作后各刷新一次，输入不被轮询打断。

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "@/shared/store";
import { Modal } from "@/shared/components/Modal";
import {
  BTN,
  BTN_DANGER_SM,
  BTN_PRIMARY,
  BTN_SM,
  INPUT,
  INPUT_MONO,
  SELECT,
} from "@/shared/lib/ui";
import * as cmd from "@/shared/commands";
import type { ModelConfigView, ModelPreset, ModelProviderView } from "@/shared/types";

const EFFORTS = ["minimal", "low", "medium", "high", "xhigh"];
const BUILTIN = "openai";

// ============ 供应商编辑弹窗 ============

function ProviderModal({
  open,
  editing,
  onClose,
  onSaved,
}: {
  open: boolean;
  editing: ModelProviderView | null;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const toast = useAppStore((s) => s.toast);
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [authMode, setAuthMode] = useState<"env" | "key" | "none">("env");
  const [envKey, setEnvKey] = useState("");
  const [token, setToken] = useState("");

  // 打开时装载编辑目标（或清空为新增）
  useEffect(() => {
    if (!open) return;
    setId(editing?.id ?? "");
    setName(editing?.name ?? "");
    setBaseUrl(editing?.baseUrl ?? "");
    setEnvKey(editing?.envKey ?? "");
    setToken(editing?.bearerToken ?? "");
    setAuthMode(
      editing ? (editing.envKey ? "env" : editing.bearerToken ? "key" : "none") : "env",
    );
  }, [open, editing]);

  const submit = async () => {
    try {
      await cmd.modelProviderSave({
        id: id.trim(),
        name: name.trim(),
        baseUrl: baseUrl.trim(),
        envKey: authMode === "env" ? envKey.trim() : "",
        bearerToken: authMode === "key" ? token.trim() : "",
      });
      toast(t("Provider saved"), "success");
      await onSaved();
      onClose();
    } catch (e) {
      toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    }
  };

  return (
    <Modal open={open} onOverlayClick={onClose} cardStyle={{ width: 560 }}>
      <h3 className="text-sm font-semibold">{editing ? t("Edit Provider") : t("Add Provider")}</h3>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-xs font-medium">ID</label>
          <input
            type="text"
            className={INPUT_MONO}
            placeholder="deepseek"
            value={id}
            disabled={!!editing}
            onChange={(e) => setId(e.target.value)}
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Display name")}</label>
          <input type="text" className={INPUT} placeholder="DeepSeek" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium">{t("Base URL")}</label>
        <input
          type="text"
          className={INPUT_MONO}
          placeholder="https://api.deepseek.com/v1"
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
        />
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium">{t("Authentication")}</label>
        <select className={SELECT} value={authMode} onChange={(e) => setAuthMode(e.target.value as typeof authMode)}>
          <option value="env">{t("Environment variable name")}</option>
          <option value="key">{t("API key (written to config.toml)")}</option>
          <option value="none">{t("No auth (local endpoints)")}</option>
        </select>
      </div>
      {authMode === "env" && (
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Env var name")}</label>
          <input
            type="text"
            className={INPUT_MONO}
            placeholder="DEEPSEEK_API_KEY"
            value={envKey}
            onChange={(e) => setEnvKey(e.target.value)}
          />
        </div>
      )}
      {authMode === "key" && (
        <div>
          <label className="mb-1 block text-xs font-medium">API Key</label>
          <input type="text" className={INPUT_MONO} placeholder="sk-..." value={token} onChange={(e) => setToken(e.target.value)} />
        </div>
      )}
      <div className="mt-1 flex justify-end gap-2">
        <button className={BTN} onClick={onClose}>
          {t("Cancel")}
        </button>
        <button className={BTN_PRIMARY} onClick={() => void submit()}>
          {t("Save")}
        </button>
      </div>
    </Modal>
  );
}

// ============ 主视图 ============

export function ModelView() {
  const { t } = useTranslation();
  const toast = useAppStore((s) => s.toast);

  const [view, setView] = useState<ModelConfigView | null>(null);
  const [model, setModel] = useState("");
  const [provider, setProvider] = useState(BUILTIN);
  const [effort, setEffort] = useState("");
  const [providerModal, setProviderModal] = useState<{ open: boolean; editing: ModelProviderView | null }>({
    open: false,
    editing: null,
  });
  const [presetModal, setPresetModal] = useState(false);
  const [presetLabel, setPresetLabel] = useState("");

  const refresh = useCallback(async () => {
    try {
      const v = await cmd.modelConfigView();
      setView(v);
      setModel(v.model);
      setProvider(v.provider || BUILTIN);
      setEffort(v.effort);
    } catch (e) {
      toast(t("Failed to load model config: {{error}}", { error: String(e) }), "error");
    }
  }, [t, toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const apply = async () => {
    try {
      await cmd.modelApply(model, provider, effort);
      toast(t("Model configuration applied"), "success");
      await refresh();
    } catch (e) {
      toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    }
  };

  const deleteProvider = async (p: ModelProviderView) => {
    const ok = await ask(
      t("Delete provider {{id}}? If it is active, model_provider falls back to the built-in OpenAI.", { id: p.id }),
      { title: t("Delete Provider"), kind: "warning" },
    );
    if (!ok) return;
    try {
      await cmd.modelProviderDelete(p.id);
      toast(t("Provider deleted"), "success");
      await refresh();
    } catch (e) {
      toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    }
  };

  const savePreset = async () => {
    const preset: ModelPreset = { id: "", label: presetLabel, model, provider, effort };
    try {
      await cmd.modelPresetSave(preset);
      toast(t("Preset saved"), "success");
      setPresetModal(false);
      setPresetLabel("");
      await refresh();
    } catch (e) {
      toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    }
  };

  const applyPreset = async (p: ModelPreset) => {
    try {
      await cmd.modelApply(p.model, p.provider, p.effort);
      toast(t("Model configuration applied"), "success");
      await refresh();
    } catch (e) {
      toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    }
  };

  const deletePreset = async (p: ModelPreset) => {
    const ok = await ask(t("Delete preset {{label}}?", { label: p.label }), {
      title: t("Delete Preset"),
      kind: "warning",
    });
    if (!ok) return;
    try {
      await cmd.modelPresetDelete(p.id);
      toast(t("Preset deleted"), "success");
      await refresh();
    } catch (e) {
      toast(t("Operation failed: {{error}}", { error: String(e) }), "error");
    }
  };

  // 供应商下拉：内置 OpenAI + config.toml 中的自定义项；当前值缺失时补一个原值选项避免跳变
  const providerOptions = [
    { value: BUILTIN, label: t("OpenAI (built-in)") },
    ...(view?.providers ?? []).map((p) => ({
      value: p.id,
      label: p.name ? `${p.name} (${p.id})` : p.id,
    })),
    ...(view && view.provider && view.provider !== BUILTIN && !view.providers.some((p) => p.id === view.provider)
      ? [{ value: view.provider, label: `${view.provider} (${t("missing")})` }]
      : []),
  ];

  const authSummary = (p: ModelProviderView) =>
    p.envKey
      ? t("Env var: {{name}}", { name: p.envKey })
      : p.bearerToken
        ? t("API key configured")
        : t("No auth");

  return (
    <main className="flex-1 overflow-y-auto p-6" id="model-view">
      <h2 className="mb-4 text-base font-semibold">{t("Models")}</h2>

      {/* 当前模型 */}
      <div className="rounded-xl border border-border bg-card text-card-foreground flex flex-col gap-3 p-4">
        <div className="text-sm font-medium">{t("Active Model")}</div>
        <div className="grid grid-cols-3 gap-3">
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Model id")}</label>
            <input
              type="text"
              className={INPUT_MONO}
              placeholder="gpt-5-codex"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Provider")}</label>
            <select className={SELECT} value={provider} onChange={(e) => setProvider(e.target.value)}>
              {providerOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium">{t("Reasoning Effort")}</label>
            <select className={SELECT} value={effort} onChange={(e) => setEffort(e.target.value)}>
              <option value="">{t("Default (not set)")}</option>
              {EFFORTS.map((e) => (
                <option key={e} value={e}>
                  {e}
                </option>
              ))}
            </select>
          </div>
        </div>
        <div className="flex items-center justify-between gap-3">
          <span className="text-xs opacity-60">
            {t("Empty model id clears the key (Codex default model). Changes are written to")}{" "}
            <span className="font-mono">~/.codex/config.toml</span>
            {t(" (backed up before each write; restart Codex sessions to take effect).")}
          </span>
          <button className={BTN_PRIMARY} onClick={() => void apply()}>
            {t("Apply")}
          </button>
        </div>
      </div>

      {/* 供应商 */}
      <div className="mt-4 rounded-xl border border-border bg-card text-card-foreground flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between">
          <div className="text-sm font-medium">{t("Providers")}</div>
          <button
            className={BTN_SM}
            onClick={() => setProviderModal({ open: true, editing: null })}
          >
            {t("Add Provider")}
          </button>
        </div>
        {(view?.providers ?? []).length === 0 && (
          <div className="text-xs opacity-60">
            {t("No custom providers; Codex uses the built-in OpenAI by default.")}
          </div>
        )}
        {(view?.providers ?? []).map((p) => (
          <div key={p.id} className="flex items-start justify-between gap-3 rounded-lg border border-border p-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm">
                <span>{p.name || p.id}</span>
                {p.active && (
                  <span className="rounded-full bg-primary/15 px-2 py-0.5 text-xs text-primary">{t("Active")}</span>
                )}
              </div>
              <div className="truncate font-mono text-xs opacity-60">
                {p.id} · {p.baseUrl}
              </div>
              <div className="text-xs opacity-60">{authSummary(p)}</div>
            </div>
            <div className="flex shrink-0 gap-2">
              <button className={BTN_SM} onClick={() => setProviderModal({ open: true, editing: p })}>
                {t("Edit")}
              </button>
              <button className={BTN_DANGER_SM} onClick={() => void deleteProvider(p)}>
                {t("Delete")}
              </button>
            </div>
          </div>
        ))}
      </div>

      {/* 预设 */}
      <div className="mt-4 rounded-xl border border-border bg-card text-card-foreground flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between">
          <div className="text-sm font-medium">{t("Presets")}</div>
          <button
            className={BTN_SM}
            disabled={!model.trim()}
            title={model.trim() ? undefined : t("Configure a model above first")}
            onClick={() => setPresetModal(true)}
          >
            {t("Save current as preset")}
          </button>
        </div>
        {(view?.presets ?? []).length === 0 && (
          <div className="text-xs opacity-60">{t("No presets yet; save the model above as one.")}</div>
        )}
        {(view?.presets ?? []).map((p) => (
          <div key={p.id} className="flex items-center justify-between gap-3 rounded-lg border border-border p-3">
            <div className="min-w-0">
              <div className="text-sm">{p.label}</div>
              <div className="truncate font-mono text-xs opacity-60">
                {p.model} · {p.provider || BUILTIN}
                {p.effort ? ` · ${p.effort}` : ""}
              </div>
            </div>
            <div className="flex shrink-0 gap-2">
              <button className={BTN_SM} onClick={() => void applyPreset(p)}>
                {t("Apply")}
              </button>
              <button className={BTN_DANGER_SM} onClick={() => void deletePreset(p)}>
                {t("Delete")}
              </button>
            </div>
          </div>
        ))}
      </div>

      <ProviderModal
        open={providerModal.open}
        editing={providerModal.editing}
        onClose={() => setProviderModal({ open: false, editing: null })}
        onSaved={refresh}
      />

      {/* 存预设：只需要一个名字，模型三字段取当前表单值 */}
      <Modal open={presetModal} onOverlayClick={() => setPresetModal(false)} cardStyle={{ width: 460 }}>
        <h3 className="text-sm font-semibold">{t("Save current as preset")}</h3>
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Preset name")}</label>
          <input
            type="text"
            className={INPUT}
            placeholder={t("e.g. Fast scout")}
            value={presetLabel}
            onChange={(e) => setPresetLabel(e.target.value)}
          />
        </div>
        <div className="font-mono text-xs opacity-60">
          {model} · {provider}
          {effort ? ` · ${effort}` : ""}
        </div>
        <div className="mt-1 flex justify-end gap-2">
          <button className={BTN} onClick={() => setPresetModal(false)}>
            {t("Cancel")}
          </button>
          <button className={BTN_PRIMARY} onClick={() => void savePreset()}>
            {t("Save")}
          </button>
        </div>
      </Modal>
    </main>
  );
}
