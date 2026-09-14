// 模型配置视图：~/.codex/config.toml 模型域的可视化管理（参考 CCursor 的 Model Config）。
// config.toml 是唯一事实来源：当前模型三键（空 = 回落默认）与 [model_providers.*] 直接读写；
// 预设库（快速切换的组合）存启动器配置。无锁定语义，不进 3s 轮询：
// 进入视图与每次操作后各刷新一次，输入不被轮询打断。

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { MoreHorizontalIcon } from "lucide-react";
import { toast } from "sonner";
import { Modal } from "@/shared/components/Modal";
import { Badge } from "@/shared/components/ui/badge";
import { Button } from "@/shared/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/shared/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/components/ui/dropdown-menu";
import { Input } from "@/shared/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/components/ui/select";
import * as cmd from "@/shared/commands";
import type { ModelConfigView, ModelPreset, ModelProviderView } from "@/shared/types";

const EFFORTS = ["minimal", "low", "medium", "high", "xhigh"];
const BUILTIN = "openai";
// Radix Select 禁止空字符串 value：以哨兵承载「Default (not set)」，回写时映射回 ""
const NONE = "__none__";

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
      toast.success(t("Provider saved"));
      await onSaved();
      onClose();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    }
  };

  return (
    <Modal
      open={open}
      onRequestClose={onClose}
      cardStyle={{ width: 560 }}
      title={editing ? t("Edit Provider") : t("Add Provider")}
    >
      <h3 className="text-sm font-semibold">{editing ? t("Edit Provider") : t("Add Provider")}</h3>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-xs font-medium">ID</label>
          <Input
            type="text"
            className="font-mono tabular-nums"
            placeholder="deepseek"
            value={id}
            disabled={!!editing}
            onChange={(e) => setId(e.target.value)}
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Display name")}</label>
          <Input type="text" placeholder="DeepSeek" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium">{t("Base URL")}</label>
        <Input
          type="text"
          className="font-mono tabular-nums"
          placeholder="https://api.deepseek.com/v1"
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
        />
      </div>
      <div>
        <label className="mb-1 block text-xs font-medium">{t("Authentication")}</label>
        <Select value={authMode} onValueChange={(v) => setAuthMode(v as typeof authMode)}>
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="env">{t("Environment variable name")}</SelectItem>
            <SelectItem value="key">{t("API key (written to config.toml)")}</SelectItem>
            <SelectItem value="none">{t("No auth (local endpoints)")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {authMode === "env" && (
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Env var name")}</label>
          <Input
            type="text"
            className="font-mono tabular-nums"
            placeholder="DEEPSEEK_API_KEY"
            value={envKey}
            onChange={(e) => setEnvKey(e.target.value)}
          />
        </div>
      )}
      {authMode === "key" && (
        <div>
          <label className="mb-1 block text-xs font-medium">API Key</label>
          <Input type="text" className="font-mono tabular-nums" placeholder="sk-..." value={token} onChange={(e) => setToken(e.target.value)} />
        </div>
      )}
      <div className="mt-1 flex justify-end gap-2">
        <Button variant="outline" onClick={onClose}>
          {t("Cancel")}
        </Button>
        <Button onClick={() => void submit()}>{t("Save")}</Button>
      </div>
    </Modal>
  );
}

// ============ 主视图 ============

export function ModelView() {
  const { t } = useTranslation();

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
      toast.error(t("Failed to load model config: {{error}}", { error: String(e) }));
    }
  }, [t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const apply = async () => {
    try {
      await cmd.modelApply(model, provider, effort);
      toast.success(t("Model configuration applied"));
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
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
      toast.success(t("Provider deleted"));
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    }
  };

  const savePreset = async () => {
    const preset: ModelPreset = { id: "", label: presetLabel, model, provider, effort };
    try {
      await cmd.modelPresetSave(preset);
      toast.success(t("Preset saved"));
      setPresetModal(false);
      setPresetLabel("");
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    }
  };

  const applyPreset = async (p: ModelPreset) => {
    try {
      await cmd.modelApply(p.model, p.provider, p.effort);
      toast.success(t("Model configuration applied"));
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
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
      toast.success(t("Preset deleted"));
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
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
    <main className="flex-1 overflow-y-auto p-4 md:p-6" id="model-view">
      <h2 className="mb-4 text-base font-semibold">{t("Models")}</h2>

      {/* 当前模型 */}
      <Card className="shadow-xs">
        <CardHeader>
          <CardTitle>{t("Active Model")}</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium">{t("Model id")}</label>
              <Input
                type="text"
                className="font-mono tabular-nums"
                placeholder="gpt-5-codex"
                value={model}
                onChange={(e) => setModel(e.target.value)}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium">{t("Provider")}</label>
              <Select value={provider} onValueChange={(v) => setProvider(v)}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {providerOptions.map((o) => (
                    <SelectItem key={o.value} value={o.value}>
                      {o.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium">{t("Reasoning Effort")}</label>
              <Select value={effort === "" ? NONE : effort} onValueChange={(v) => setEffort(v === NONE ? "" : v)}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NONE}>{t("Default (not set)")}</SelectItem>
                  {EFFORTS.map((e) => (
                    <SelectItem key={e} value={e} className="tabular-nums">
                      {e}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
        </CardContent>
        <CardContent className="flex items-center justify-between gap-3">
          <span className="text-xs text-muted-foreground">
            {t("Empty model id clears the key (Codex default model). Changes are written to")}{" "}
            <span className="font-mono tabular-nums">~/.codex/config.toml</span>
            {t(" (backed up before each write; restart Codex sessions to take effect).")}
          </span>
          <Button onClick={() => void apply()}>{t("Apply")}</Button>
        </CardContent>
      </Card>

      {/* 供应商 */}
      <Card className="mt-4 shadow-xs">
        <CardHeader>
          <CardTitle>{t("Providers")}</CardTitle>
          <CardAction>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setProviderModal({ open: true, editing: null })}
            >
              {t("Add Provider")}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {(view?.providers ?? []).length === 0 && (
            <div className="text-xs text-muted-foreground">
              {t("No custom providers; Codex uses the built-in OpenAI by default.")}
            </div>
          )}
          {(view?.providers ?? []).map((p) => (
            <div key={p.id} className="flex items-start justify-between gap-3 rounded-lg border border-border p-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm">
                  <span>{p.name || p.id}</span>
                  {p.active && <Badge variant="secondary">{t("Active")}</Badge>}
                </div>
                <div className="truncate font-mono text-xs tabular-nums text-muted-foreground">
                  {p.id} · {p.baseUrl}
                </div>
                <div className="text-xs text-muted-foreground">{authSummary(p)}</div>
              </div>
              <div className="flex shrink-0 gap-2">
                <Button variant="outline" size="sm" onClick={() => setProviderModal({ open: true, editing: p })}>
                  {t("Edit")}
                </Button>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon-sm" aria-label={t("More actions")}>
                      <MoreHorizontalIcon />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem variant="destructive" onSelect={() => void deleteProvider(p)}>
                      {t("Delete")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      {/* 预设 */}
      <Card className="mt-4 shadow-xs">
        <CardHeader>
          <CardTitle>{t("Presets")}</CardTitle>
          <CardAction>
            <Button
              variant="outline"
              size="sm"
              disabled={!model.trim()}
              title={model.trim() ? undefined : t("Configure a model above first")}
              onClick={() => setPresetModal(true)}
            >
              {t("Save current as preset")}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {(view?.presets ?? []).length === 0 && (
            <div className="text-xs text-muted-foreground">{t("No presets yet; save the model above as one.")}</div>
          )}
          {(view?.presets ?? []).map((p) => (
            <div key={p.id} className="flex items-center justify-between gap-3 rounded-lg border border-border p-3">
              <div className="min-w-0">
                <div className="text-sm">{p.label}</div>
                <div className="truncate font-mono text-xs tabular-nums text-muted-foreground">
                  {p.model} · {p.provider || BUILTIN}
                  {p.effort ? ` · ${p.effort}` : ""}
                </div>
              </div>
              <div className="flex shrink-0 gap-2">
                <Button variant="outline" size="sm" onClick={() => void applyPreset(p)}>
                  {t("Apply")}
                </Button>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon-sm" aria-label={t("More actions")}>
                      <MoreHorizontalIcon />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem variant="destructive" onSelect={() => void deletePreset(p)}>
                      {t("Delete")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <ProviderModal
        open={providerModal.open}
        editing={providerModal.editing}
        onClose={() => setProviderModal({ open: false, editing: null })}
        onSaved={refresh}
      />

      {/* 存预设：只需要一个名字，模型三字段取当前表单值 */}
      <Modal
        open={presetModal}
        onRequestClose={() => setPresetModal(false)}
        cardStyle={{ width: 460 }}
        title={t("Save current as preset")}
      >
        <h3 className="text-sm font-semibold">{t("Save current as preset")}</h3>
        <div>
          <label className="mb-1 block text-xs font-medium">{t("Preset name")}</label>
          <Input
            type="text"
            placeholder={t("e.g. Fast scout")}
            value={presetLabel}
            onChange={(e) => setPresetLabel(e.target.value)}
          />
        </div>
        <div className="font-mono text-xs tabular-nums text-muted-foreground">
          {model} · {provider}
          {effort ? ` · ${effort}` : ""}
        </div>
        <div className="mt-1 flex justify-end gap-2">
          <Button variant="outline" onClick={() => setPresetModal(false)}>
            {t("Cancel")}
          </Button>
          <Button onClick={() => void savePreset()}>{t("Save")}</Button>
        </div>
      </Modal>
    </main>
  );
}
