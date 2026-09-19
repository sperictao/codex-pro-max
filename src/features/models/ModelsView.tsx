// 模型配置工作台：编辑 ~/.codex/config.toml 的模型域。
//
// 与旧版 ModelView 的差别不在写入语义（三键 + [model_providers.*] 没变），
// 而在可见面：供应商段现在覆盖 Codex 的全部托管键（wire_api、headers、
// query_params、重试与超时、requires_openai_auth…），未文档化的新键原样透传；
// 另加 models.dev 目录、环境变量就绪状态、连通性验证与本机导入。
//
// 主页面只放摘要、状态与快捷操作，完整字段进入 ProviderDialog 渐进披露。
// config.toml 仍是唯一事实来源：视图在进入页面与每次操作后现读，不轮询。

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  DownloadIcon,
  MoreHorizontalIcon,
  PencilIcon,
  RefreshCwIcon,
  SearchIcon,
} from "lucide-react";
import { toast } from "sonner";
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
import { Modal } from "@/shared/components/Modal";
import * as cmd from "@/shared/commands";
import type { CatalogView, ModelConfigView, ModelPreset, ProviderSchema } from "@/shared/types";
import { ImportDialog } from "./ImportDialog";
import { ModelPickerDialog } from "./ModelPickerDialog";
import { ProviderDialog } from "./ProviderDialog";
import {
  BUILTIN_PROVIDER,
  EMPTY_CONFIG,
  NONE,
  authSummary,
  catalogModel,
  credentialSourceKey,
  effortSupported,
  fmtTokens,
  modelStatus,
  presetSummary,
  providerChoices,
  removeProvider,
  upsertProvider,
} from "./ops";

/** 目录缺失时的占位（用于本文件内的可选参数默认值） */
const NO_CATALOG: CatalogView = EMPTY_CONFIG.catalog;

export function ModelsView() {
  const { t } = useTranslation();

  const [view, setView] = useState<ModelConfigView>(EMPTY_CONFIG);
  const [loading, setLoading] = useState(true);
  const [model, setModel] = useState("");
  const [provider, setProvider] = useState(BUILTIN_PROVIDER);
  const [effort, setEffort] = useState("");
  const [providerDialog, setProviderDialog] = useState<{
    open: boolean;
    editing: ProviderSchema | null;
  }>({ open: false, editing: null });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [presetModal, setPresetModal] = useState(false);
  const [presetLabel, setPresetLabel] = useState("");
  const [refreshingCatalog, setRefreshingCatalog] = useState(false);
  /** 每个路由的最近一次连通性验证结果（会话内即时反馈，不落盘） */
  const [probes, setProbes] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    try {
      const next = await cmd.modelConfigView();
      setView(next);
      setModel(next.active.model);
      setProvider(next.active.provider || BUILTIN_PROVIDER);
      setEffort(next.active.effort);
    } catch (e) {
      toast.error(t("Failed to load model config: {{error}}", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  /** 目录刷新只影响目录，不动用户正在编辑的三个字段 */
  const refreshCatalog = useCallback(async () => {
    setRefreshingCatalog(true);
    try {
      const catalog = await cmd.modelCatalogRefresh();
      setView((prev) => ({ ...prev, catalog }));
    } catch (e) {
      toast.error(t("Failed to refresh the catalog: {{error}}", { error: String(e) }));
    } finally {
      setRefreshingCatalog(false);
    }
  }, [t]);

  const apply = async () => {
    try {
      await cmd.modelApply(model, provider, effort);
      toast.success(t("Model configuration applied"));
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    }
  };

  const deleteProvider = async (p: ProviderSchema) => {
    const ok = await ask(
      t(
        "Delete provider {{id}}? If it is active, model_provider falls back to the built-in OpenAI.",
        { id: p.route },
      ),
      { title: t("Delete Provider"), kind: "warning" },
    );
    if (!ok) return;
    try {
      await cmd.modelProviderDelete(p.route);
      setView((prev) => removeProvider(prev, p.route));
      toast.success(t("Provider deleted"));
      await refresh();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    }
  };

  const testProvider = async (p: ProviderSchema) => {
    setProbes((prev) => ({ ...prev, [p.route]: t("Testing…") }));
    try {
      const result = await cmd.modelTestConnection(p);
      setProbes((prev) => ({
        ...prev,
        [p.route]: result.ok
          ? [
              t("Reachable"),
              result.status !== null ? t("HTTP {{status}}", { status: result.status }) : null,
              result.modelCount !== null
                ? t("{{count}} models", { count: result.modelCount })
                : null,
            ]
              .filter(Boolean)
              .join(" · ")
          : (result.error ?? t("Unreachable")),
      }));
    } catch (e) {
      setProbes((prev) => ({ ...prev, [p.route]: String(e) }));
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

  const choices = useMemo(() => providerChoices(view, t), [view, t]);
  const status = useMemo(
    () => modelStatus(view.catalog, provider, model.trim()),
    [view.catalog, provider, model],
  );
  /** 当前路由的目录条目（用于容量提示与选模型） */
  const entries = view.catalog.providers[provider] ?? [];
  const selectedEntry = catalogModel(view.catalog, provider, model.trim());
  const effortOk = effortSupported(status, effort);

  /** credential 名 → 就绪信息，供供应商行与表单共用 */
  const credentialOf = (name: string | null) =>
    name ? view.credentials.find((c) => c.name === name)?.info : undefined;

  const activeAuthNeedsAttention = (() => {
    const current = view.providers.find((p) => p.route === view.active.provider);
    if (!current?.envKey) return false;
    const info = credentialOf(current.envKey);
    return info !== undefined && !info.configured;
  })();

  return (
    <main className="app-page-scroll flex-1 overflow-y-auto" id="model-view">
      <h2 className="mb-4 text-base font-semibold">{t("Models")}</h2>

      {/* 当前模型 */}
      <Card className="shadow-xs">
        <CardHeader>
          <CardTitle>{t("Active Model")}</CardTitle>
          <CardAction>
            <span className="text-xs text-muted-foreground">
              {view.catalog.missing
                ? t("Catalog not downloaded")
                : view.catalog.stale
                  ? t("Catalog is out of date")
                  : t("Catalog is up to date")}
            </span>
          </CardAction>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium" htmlFor="active-model-id">
                {t("Model id")}
              </label>
              <div className="flex items-center gap-1.5">
                <Input
                  id="active-model-id"
                  className="font-mono tabular-nums"
                  placeholder="gpt-5-codex"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                />
                <Button
                  variant="outline"
                  size="icon"
                  aria-label={t("Pick a model")}
                  onClick={() => setPickerOpen(true)}
                >
                  <SearchIcon />
                </Button>
              </div>
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium" htmlFor="active-provider">
                {t("Provider")}
              </label>
              <Select value={provider} onValueChange={setProvider}>
                <SelectTrigger
                  id="active-provider"
                  aria-label={t("Provider")}
                  className="w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {choices.map((o) => (
                    <SelectItem key={o.value} value={o.value}>
                      {o.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium" htmlFor="active-effort">
                {t("Reasoning Effort")}
              </label>
              <Select
                value={effort === "" ? NONE : effort}
                onValueChange={(v) => setEffort(v === NONE ? "" : v)}
              >
                <SelectTrigger
                  id="active-effort"
                  aria-label={t("Reasoning Effort")}
                  className="w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NONE}>{t("Default (not set)")}</SelectItem>
                  {view.efforts.map((e) => (
                    <SelectItem key={e} value={e} className="tabular-nums">
                      {e}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {/* 目录只做提示，不拦截：本地端点的模型按定义不在目录里 */}
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {selectedEntry ? (
              <>
                <Badge variant="secondary" className="font-mono">
                  {fmtTokens(selectedEntry.context)}
                </Badge>
                <span>{t("Context window")}</span>
                {selectedEntry.maxTokens !== null && (
                  <span>
                    {t("max output")}{" "}
                    <span className="font-mono tabular-nums">
                      {fmtTokens(selectedEntry.maxTokens)}
                    </span>
                  </span>
                )}
                {selectedEntry.reasoning && <span>{t("reasoning")}</span>}
              </>
            ) : status.kind === "unknown" ? (
              <span>{t("This model id is not listed in the catalog for the selected provider.")}</span>
            ) : status.kind === "unlisted" ? (
              <span>{t("The catalog does not cover this provider; metadata is unavailable.")}</span>
            ) : status.kind === "no-catalog" ? (
              <span>
                {t("Download the catalog to see context window and reasoning support.")}
              </span>
            ) : null}
            {entries.length > 0 && !selectedEntry && (
              <span>{t("{{count}} models available for this provider", { count: entries.length })}</span>
            )}
            {!effortOk && (
              <span className="text-destructive">
                {t("The selected model does not advertise this reasoning level.")}
              </span>
            )}
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

      {activeAuthNeedsAttention && (
        <p className="mt-2 text-xs text-destructive">
          {t("The active provider references an environment variable that is not set.")}
        </p>
      )}

      {/* 供应商 */}
      <Card className="mt-4 shadow-xs">
        <CardHeader>
          <CardTitle>{t("Providers")}</CardTitle>
          <CardAction className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void refreshCatalog()}
              disabled={refreshingCatalog}
            >
              <RefreshCwIcon />
              {refreshingCatalog ? t("Refreshing…") : t("Refresh catalog")}
            </Button>
            <Button variant="outline" size="sm" onClick={() => setImportOpen(true)}>
              <DownloadIcon />
              {t("Import providers")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setProviderDialog({ open: true, editing: null })}
            >
              {t("Add Provider")}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {!loading && view.providers.length === 0 && (
            <div className="text-xs text-muted-foreground">
              {t("No custom providers; Codex uses the built-in OpenAI by default.")}
            </div>
          )}
          {view.providers.map((p) => {
            const info = credentialOf(p.envKey);
            const missingCredential = p.envKey !== null && info !== undefined && !info.configured;
            return (
              <div
                key={p.route}
                className="flex items-start justify-between gap-3 rounded-lg border border-border p-3"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm">
                    <span>{p.name || p.route}</span>
                    {p.route === view.active.provider && (
                      <Badge variant="secondary">{t("Active")}</Badge>
                    )}
                    {p.wireApi && (
                      <Badge variant="secondary" className="font-mono">
                        {p.wireApi}
                      </Badge>
                    )}
                    {missingCredential && (
                      <Badge variant="destructive">{t("Missing credential")}</Badge>
                    )}
                  </div>
                  <div className="truncate font-mono text-xs tabular-nums text-muted-foreground">
                    {p.route} · {p.baseURL ?? "—"}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {authSummary(p, t)}
                    {p.envKey !== null && info !== undefined && (
                      <>
                        {" · "}
                        {info.configured
                          ? t("resolved from {{source}}", {
                              source: t(credentialSourceKey(info.source)),
                            })
                          : t("not set")}
                      </>
                    )}
                  </div>
                  {probes[p.route] && (
                    <div className="mt-1 font-mono text-xs tabular-nums text-muted-foreground">
                      {probes[p.route]}
                    </div>
                  )}
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button variant="outline" size="sm" onClick={() => void testProvider(p)}>
                    {t("Test")}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setProviderDialog({ open: true, editing: p })}
                  >
                    <PencilIcon />
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
            );
          })}
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
          {view.presets.length === 0 && (
            <div className="text-xs text-muted-foreground">
              {t("No presets yet; save the model above as one.")}
            </div>
          )}
          {view.presets.map((p) => (
            <div
              key={p.id}
              className="flex items-center justify-between gap-3 rounded-lg border border-border p-3"
            >
              <div className="min-w-0">
                <div className="text-sm">{p.label}</div>
                <div className="truncate font-mono text-xs tabular-nums text-muted-foreground">
                  {presetSummary(p)}
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

      <ProviderDialog
        open={providerDialog.open}
        editing={providerDialog.editing}
        credentialOf={credentialOf}
        onClose={() => setProviderDialog({ open: false, editing: null })}
        onSaved={(saved, previousRoute) => {
          setView((prev) => upsertProvider(prev, saved, previousRoute));
          // 保存后回读磁盘：凭据就绪等派生状态以 config.toml 现读为准
          void refresh();
        }}
        onCredentialsChanged={() => void refresh()}
      />

      <ModelPickerDialog
        open={pickerOpen}
        catalog={view.catalog ?? NO_CATALOG}
        route={provider}
        onPick={(id) => setModel(id)}
        onClose={() => setPickerOpen(false)}
        onRefresh={refreshCatalog}
      />

      <ImportDialog
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={refresh}
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
          <label className="mb-1 block text-xs font-medium" htmlFor="preset-label">
            {t("Preset name")}
          </label>
          <Input
            id="preset-label"
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
