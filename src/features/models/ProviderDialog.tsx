// features/models/ProviderDialog：单个 `[model_providers.<id>]` 段的完整编辑面板。
// 渐进披露：主页面只显示摘要，这里才展开全部托管字段。Codex 新增的未知键
// 只读列出——它们在保存时原样写回，不让用户以为"看不见就是没有"。

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { PlugZapIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/shared/components/ui/badge";
import { Button } from "@/shared/components/ui/button";
import { Input } from "@/shared/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/components/ui/select";
import { Switch } from "@/shared/components/ui/switch";
import { Modal } from "@/shared/components/Modal";
import * as cmd from "@/shared/commands";
import type { ConnectionResult, CredentialInfo, ProviderSchema } from "@/shared/types";
import {
  HeadersEditor,
  recordFromRows,
  rowsFromRecord,
  type HeaderRow,
} from "./HeadersEditor";
import {
  authModeOf,
  credentialSourceKey,
  emptyProvider,
  prepareProvider,
  type AuthMode,
} from "./ops";

/** wire 协议：缺省（chat）与显式 chat 等价，故空串不单列选项 */
const WIRE_APIS = ["chat", "responses"];

/** 数值输入的空串 = 未设置（删键），有值则转数字 */
function numOrNull(raw: string): number | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

function numToInput(value: number | null): string {
  return value === null ? "" : String(value);
}

export function ProviderDialog({
  open,
  editing,
  credentialOf,
  onClose,
  onSaved,
  onCredentialsChanged,
}: {
  open: boolean;
  /** null = 新增 */
  editing: ProviderSchema | null;
  /** 变量名 → 就绪信息（来自模型页视图）；查不到（如新建供应商的新变量）则不显示就绪区块 */
  credentialOf?: (name: string | null) => CredentialInfo | undefined;
  onClose: () => void;
  onSaved: (saved: ProviderSchema, previousRoute: string | null) => void;
  /** 用户级 .env 写入成功后回调（模型页借此回读凭据状态） */
  onCredentialsChanged?: () => void;
}) {
  const { t } = useTranslation();

  const [draft, setDraft] = useState<ProviderSchema>(() => editing ?? emptyProvider());
  const [authMode, setAuthMode] = useState<AuthMode>("env");
  const [token, setToken] = useState("");
  const [queryRows, setQueryRows] = useState<HeaderRow[]>([]);
  const [headerRows, setHeaderRows] = useState<HeaderRow[]>([]);
  const [envHeaderRows, setEnvHeaderRows] = useState<HeaderRow[]>([]);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ConnectionResult | null>(null);
  /** 用户级 .env 的待写入值（env_key 所指变量的明文，只上行不下行） */
  const [envValue, setEnvValue] = useState("");
  const [savingEnvValue, setSavingEnvValue] = useState(false);

  // 打开时装载编辑目标（或清空为新增）
  useEffect(() => {
    if (!open) return;
    const next = editing ? { ...editing } : emptyProvider();
    setDraft(next);
    setAuthMode(authModeOf(next));
    // 密钥不回传：编辑已有供应商时输入框留空，留空 = 保持磁盘原值
    setToken("");
    setEnvValue("");
    setQueryRows(rowsFromRecord(next.queryParams));
    setHeaderRows(rowsFromRecord(next.httpHeaders));
    setEnvHeaderRows(rowsFromRecord(next.envHttpHeaders));
    setTestResult(null);
    // 有高级字段时默认展开，避免用户以为配置丢了
    setAdvancedOpen(
      next.requestMaxRetries !== null ||
        next.streamMaxRetries !== null ||
        next.streamIdleTimeoutMs !== null ||
        next.startupTimeoutMs !== null ||
        next.toolTimeoutSec !== null,
    );
  }, [open, editing]);

  const patch = (next: Partial<ProviderSchema>) => setDraft((prev) => ({ ...prev, ...next }));

  /**
   * 当前草稿 + 认证方式 → 校验结论、下发载荷与密钥下发值。
   * 三者出自同一份输入，避免"校验说合法、下发却清空了密钥"这类错位。
   */
  const {
    payload,
    bearerToken,
    error: errorKey,
  } = useMemo(
    () =>
      prepareProvider(
        {
          ...draft,
          // 输入框里的新密钥并入草稿：prepareProvider 据此校验并产出下发值
          bearerToken: token.trim(),
          queryParams: recordFromRows(queryRows),
          httpHeaders: recordFromRows(headerRows),
          envHttpHeaders: recordFromRows(envHeaderRows),
        },
        authMode,
        token.trim() !== "",
      ),
    [draft, authMode, token, queryRows, headerRows, envHeaderRows],
  );

  const newKeyProvided = token.trim() !== "";

  /** env_key 所指变量的就绪信息；视图未覆盖的新变量没有信息，区块整体隐藏 */
  const envKeyInfo =
    draft.envKey && draft.envKey.trim() ? credentialOf?.(draft.envKey.trim()) : undefined;

  const saveEnvValue = async () => {
    const name = draft.envKey?.trim();
    const value = envValue.trim();
    if (!name || !value) return;
    setSavingEnvValue(true);
    try {
      await cmd.modelCredentialSet(name, value);
      setEnvValue("");
      toast.success(t("Credential stored in your user .env."));
      await onCredentialsChanged?.();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    } finally {
      setSavingEnvValue(false);
    }
  };

  const save = async () => {
    if (errorKey) {
      toast.error(t(errorKey));
      return;
    }
    // 密钥下发三态：非 key 模式 = ""（显式清除磁盘密钥，认证二选一）；
    // key 模式本次填了值 = 新值；key 模式留空 = null（保持磁盘原值）
    const bearer = authMode === "key" && !newKeyProvided ? null : bearerToken;
    // 明文只上行不下行：载荷与入库视图都不携带 bearerToken
    const saved = { ...payload, bearerToken: undefined };
    try {
      await cmd.modelProviderSave(saved, bearer, editing?.unknownKeys ?? []);
      toast.success(t("Provider saved"));
      onSaved(saved, editing?.route ?? null);
      onClose();
    } catch (e) {
      toast.error(t("Operation failed: {{error}}", { error: String(e) }));
    }
  };

  const test = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      // 用未保存的草稿测：用户不必先落盘再验证
      const probe = { ...payload, bearerToken: authMode === "key" ? token.trim() : "" };
      setTestResult(await cmd.modelTestConnection(probe));
    } catch (e) {
      setTestResult({
        ok: false,
        endpoint: payload.baseURL ?? "",
        authenticated: false,
        status: null,
        modelCount: null,
        error: String(e),
      });
    } finally {
      setTesting(false);
    }
  };

  return (
    <Modal
      open={open}
      onRequestClose={onClose}
      cardStyle={{ width: 640 }}
      title={editing ? t("Edit Provider") : t("Add Provider")}
    >
      <h3 className="text-sm font-semibold">
        {editing ? t("Edit Provider") : t("Add Provider")}
      </h3>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-xs font-medium" htmlFor="provider-route">
            {t("Provider id")}
          </label>
          <Input
            id="provider-route"
            className="font-mono tabular-nums"
            placeholder="deepseek"
            value={draft.route}
            disabled={!!editing}
            onChange={(e) => patch({ route: e.target.value })}
          />
          <p className="mt-1 text-xs text-muted-foreground">
            {editing
              ? t("Provider id is the table key and cannot be changed after creation.")
              : t("Letters, digits, '-' and '_'. 'openai' is reserved.")}
          </p>
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium" htmlFor="provider-name">
            {t("Display name")}
          </label>
          <Input
            id="provider-name"
            placeholder="DeepSeek"
            value={draft.name ?? ""}
            onChange={(e) => patch({ name: e.target.value })}
          />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-xs font-medium" htmlFor="provider-base-url">
            {t("Base URL")}
          </label>
          <Input
            id="provider-base-url"
            className="font-mono tabular-nums"
            placeholder="https://api.deepseek.com/v1"
            value={draft.baseURL ?? ""}
            onChange={(e) => patch({ baseURL: e.target.value })}
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium" htmlFor="provider-wire-api">
            {t("Wire API")}
          </label>
          <Select
            value={draft.wireApi ?? ""}
            onValueChange={(v) => patch({ wireApi: v === "" ? null : v })}
          >
            <SelectTrigger id="provider-wire-api" className="w-full">
              <SelectValue placeholder={t("chat (default)")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="chat">{t("chat (default)")}</SelectItem>
              {WIRE_APIS.filter((api) => api !== "chat").map((api) => (
                <SelectItem key={api} value={api}>
                  {api}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <div>
        <label className="mb-1 block text-xs font-medium" htmlFor="provider-auth-mode">
          {t("Authentication")}
        </label>
        <Select value={authMode} onValueChange={(v) => setAuthMode(v as AuthMode)}>
          <SelectTrigger id="provider-auth-mode" aria-label={t("Authentication")} className="w-full">
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
          <label className="mb-1 block text-xs font-medium" htmlFor="provider-env-key">
            {t("Env var name")}
          </label>
          <Input
            id="provider-env-key"
            className="font-mono tabular-nums"
            placeholder="DEEPSEEK_API_KEY"
            value={draft.envKey ?? ""}
            onChange={(e) => patch({ envKey: e.target.value })}
          />
          <p className="mt-1 text-xs text-muted-foreground">
            {t("Only the variable name is stored; the key itself stays in your environment.")}
          </p>
          {envKeyInfo && (
            <>
              <p className="mt-1 text-xs text-muted-foreground">
                {envKeyInfo.configured
                  ? t("resolved from {{source}}", {
                      source: t(credentialSourceKey(envKeyInfo.source)),
                    })
                  : t("not set")}
                {!envKeyInfo.writable && (
                  <>
                    {" · "}
                    {t("read-only")}
                  </>
                )}
              </p>
              {envKeyInfo.writable && (
                <div className="mt-1.5 flex items-center gap-1.5">
                  <Input
                    id="provider-env-value"
                    type="password"
                    className="font-mono tabular-nums"
                    placeholder={t("Value to store in ~/.codex/.env")}
                    aria-label={t("Value to store in ~/.codex/.env")}
                    value={envValue}
                    onChange={(e) => setEnvValue(e.target.value)}
                  />
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={!envValue.trim() || savingEnvValue}
                    onClick={() => void saveEnvValue()}
                  >
                    {savingEnvValue ? t("Saving…") : t("Save to .env")}
                  </Button>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {authMode === "key" && (
        <div>
          <label className="mb-1 block text-xs font-medium" htmlFor="provider-token">
            {t("API key (written to config.toml)")}
          </label>
          <Input
            id="provider-token"
            type="password"
            className="font-mono tabular-nums"
            placeholder={editing?.hasBearerToken ? t("Leave empty to keep the stored key") : "sk-..."}
            value={token}
            onChange={(e) => setToken(e.target.value)}
          />
          <p className="mt-1 text-xs text-muted-foreground">
            {t("Stored in plain text in config.toml; prefer an environment variable when possible.")}
          </p>
        </div>
      )}

      <details
        className="rounded-lg border border-border p-3"
        open={advancedOpen}
        onToggle={(e) => setAdvancedOpen((e.target as HTMLDetailsElement).open)}
      >
        <summary className="cursor-pointer text-xs font-medium select-none">
          {t("Advanced")}
        </summary>
        <div className="mt-3 flex flex-col gap-3">
          <div className="grid grid-cols-3 gap-3">
            {(
              [
                ["requestMaxRetries", "Request retries", "4"],
                ["streamMaxRetries", "Stream retries", "5"],
                ["streamIdleTimeoutMs", "Stream idle timeout (ms)", "300000"],
              ] as const
            ).map(([field, label, placeholder]) => (
              <div key={field}>
                <label className="mb-1 block text-xs font-medium" htmlFor={`provider-${field}`}>
                  {t(label)}
                </label>
                <Input
                  id={`provider-${field}`}
                  inputMode="numeric"
                  className="tabular-nums"
                  placeholder={placeholder}
                  value={numToInput(draft[field])}
                  onChange={(e) => {
                    const raw = e.target.value.trim();
                    if (raw !== "" && !/^\d+$/.test(raw)) return;
                    patch({ [field]: numOrNull(raw) } as Partial<ProviderSchema>);
                  }}
                />
              </div>
            ))}
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium" htmlFor="provider-startup-timeout">
                {t("Startup timeout (ms)")}
              </label>
              <Input
                id="provider-startup-timeout"
                inputMode="numeric"
                className="tabular-nums"
                placeholder="10000"
                value={numToInput(draft.startupTimeoutMs)}
                onChange={(e) => {
                  const raw = e.target.value.trim();
                  if (raw !== "" && !/^\d+$/.test(raw)) return;
                  patch({ startupTimeoutMs: numOrNull(raw) });
                }}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium" htmlFor="provider-tool-timeout">
                {t("Tool timeout (sec)")}
              </label>
              <Input
                id="provider-tool-timeout"
                inputMode="decimal"
                className="tabular-nums"
                placeholder="60"
                value={numToInput(draft.toolTimeoutSec)}
                onChange={(e) => {
                  const raw = e.target.value.trim();
                  if (raw !== "" && !/^\d*\.?\d*$/.test(raw)) return;
                  patch({ toolTimeoutSec: numOrNull(raw) });
                }}
              />
            </div>
          </div>

          <HeadersEditor
            id="provider-http-headers"
            label={t("Static headers")}
            hint={t("Sent with every request, values are literal.")}
            keyPlaceholder="X-Example-Header"
            valuePlaceholder="example-value"
            rows={headerRows}
            onChange={setHeaderRows}
          />
          <HeadersEditor
            id="provider-env-headers"
            label={t("Headers from environment")}
            hint={t("The value column holds an environment variable name.")}
            keyPlaceholder="X-Example-Features"
            valuePlaceholder="EXAMPLE_FEATURES"
            rows={envHeaderRows}
            onChange={setEnvHeaderRows}
          />
          <HeadersEditor
            id="provider-query-params"
            label={t("Query parameters")}
            hint={t("Appended to the request URL (e.g. Azure api-version).")}
            keyPlaceholder="api-version"
            valuePlaceholder="2025-04-01-preview"
            rows={queryRows}
            onChange={setQueryRows}
          />

          <div className="flex flex-col gap-2">
            {(
              [
                ["requiresOpenaiAuth", "Requires OpenAI auth"],
                ["supportsWebsockets", "Supports websockets"],
              ] as const
            ).map(([field, label]) => (
              <label key={field} className="flex items-center gap-2 text-xs font-medium">
                <Switch
                  checked={draft[field] === true}
                  onCheckedChange={(checked) =>
                    patch({ [field]: checked ? true : null } as Partial<ProviderSchema>)
                  }
                />
                {t(label)}
              </label>
            ))}
          </div>

          {draft.unknownKeys.length > 0 && (
            <div>
              <p className="mb-1 text-xs font-medium">{t("Keys kept as-is")}</p>
              <p className="mb-1.5 text-xs text-muted-foreground">
                {t("These keys are not managed here and are written back unchanged.")}
              </p>
              <div className="flex flex-wrap gap-1">
                {draft.unknownKeys.map((key) => (
                  <Badge key={key} variant="secondary" className="font-mono">
                    {key}
                  </Badge>
                ))}
              </div>
            </div>
          )}
        </div>
      </details>

      {testResult && (
        <div className="rounded-lg border border-border p-3 text-xs">
          <div className="flex items-center gap-2">
            <Badge variant={testResult.ok ? "secondary" : "destructive"}>
              {testResult.ok ? t("Reachable") : t("Unreachable")}
            </Badge>
            {testResult.status !== null && (
              <span className="tabular-nums text-muted-foreground">
                {t("HTTP {{status}}", { status: testResult.status })}
              </span>
            )}
            {testResult.modelCount !== null && (
              <span className="tabular-nums text-muted-foreground">
                {t("{{count}} models", { count: testResult.modelCount })}
              </span>
            )}
            {!testResult.authenticated && testResult.ok && (
              <span className="text-muted-foreground">{t("no credentials were sent")}</span>
            )}
          </div>
          <p className="mt-1 truncate font-mono tabular-nums text-muted-foreground">
            {testResult.endpoint}
          </p>
          {testResult.error && <p className="mt-1 text-destructive">{testResult.error}</p>}
        </div>
      )}

      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={() => void test()} disabled={testing}>
          <PlugZapIcon />
          {testing ? t("Testing…") : t("Test connection")}
        </Button>
        <Button variant="outline" onClick={onClose}>
          {t("Cancel")}
        </Button>
        <Button onClick={() => void save()}>{t("Save")}</Button>
      </div>
    </Modal>
  );
}
