import React, { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type LlmConnection,
  type LlmModelConfig,
  type PostProcessProvider,
} from "@/bindings";
import { ChevronDown, ChevronRight, RefreshCw, Trash2 } from "lucide-react";
import { useSettings } from "../../../hooks/useSettings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Select } from "../../ui/Select";
import { Input } from "../../ui/Input";
import { Button } from "../../ui/Button";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import Badge from "../../ui/Badge";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";

/// Vorlagen, bei denen ein Schluessel nichts zu suchen hat: lokal, ohne Konto.
const KEYLESS_KINDS = new Set([
  "ollama",
  "vllm",
  "local",
  "apple_intelligence",
]);

/**
 * Verbindungen zu Sprachmodell-Anbietern und die Freigabe ihrer Modelle.
 *
 * Eine Verbindung ist ein Konto bei einem Anbieter oder eine lokale
 * Laufzeit; von einer Vorlage darf es mehrere geben. Nur freigegebene
 * Modelle erscheinen in der Auswahl der App — ein Anbieter listet Dutzende,
 * von denen man zwei braucht. Das aktive Modell steht oben, weil es die eine
 * Wahl ist, die man im Alltag trifft; alles darunter ist Einrichtung.
 */
export const LlmConnectionsSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();

  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
  const templates = (getSetting("post_process_providers") ??
    []) as PostProcessProvider[];
  const apiKeys = (getSetting("post_process_api_keys") ?? {}) as Record<
    string,
    string
  >;

  const [error, setError] = useState<string | null>(null);

  const run = useCallback(
    async (call: () => Promise<{ status: string; error?: unknown }>) => {
      setError(null);
      const result = await call();
      if (result.status === "error") {
        setError(String(result.error));
        return false;
      }
      await refreshSettings();
      return true;
    },
    [refreshSettings],
  );

  // Aktives Modell: nur freigegebene Modelle eingeschalteter Verbindungen.
  const selectable = useMemo(() => {
    const on = new Set(
      connections.filter((c) => c.enabled !== false).map((c) => c.id),
    );
    return models.filter((m) => m.enabled !== false && on.has(m.connection_id));
  }, [connections, models]);

  const labelFor = (m: LlmModelConfig) => {
    const conn = connections.find((c) => c.id === m.connection_id);
    return conn ? `${m.label} · ${conn.label}` : m.label;
  };

  const addConnection = async (kind: string) => {
    const template = templates.find((p) => p.id === kind);
    if (!template) return;
    // Kennung aus Vorlage und Zeit: lesbar in der Einstellungsdatei und
    // eindeutig, auch wenn man dieselbe Vorlage zweimal anlegt.
    const id = `${kind}-${Date.now().toString(36)}`;
    await run(() =>
      commands.llmUpsertConnection({
        id,
        kind,
        label: template.label,
        base_url: template.base_url,
        enabled: true,
      }),
    );
  };

  const templateOptions = templates.map((p) => ({
    value: p.id,
    label: p.label,
  }));

  return (
    <div className="space-y-6">
      <SettingsGroup title={t("settings.llm.active.title")}>
        <SettingContainer
          title={t("settings.llm.active.model")}
          description={t("settings.llm.active.description")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <div className="w-72">
            {selectable.length > 0 ? (
              <Select
                value={activeId ?? ""}
                options={selectable.map((m) => ({
                  value: m.id,
                  label: labelFor(m),
                }))}
                placeholder={t("settings.llm.active.placeholder")}
                isClearable={false}
                onChange={(value) => {
                  if (value) void run(() => commands.llmSetActiveModel(value));
                }}
              />
            ) : (
              <span className="text-sm text-text/60">
                {t("settings.llm.active.none")}
              </span>
            )}
          </div>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup title={t("settings.llm.connections.title")}>
        <div className="px-4 py-3 space-y-3">
          {error && (
            <p className="text-sm text-red-500 break-words" role="alert">
              {error}
            </p>
          )}
          {connections.length === 0 && (
            <p className="text-sm text-text/60">
              {t("settings.llm.connections.empty")}
            </p>
          )}
          {connections.map((connection) => (
            <ConnectionRow
              key={connection.id}
              connection={connection}
              template={templates.find((p) => p.id === connection.kind)}
              models={models.filter((m) => m.connection_id === connection.id)}
              apiKey={apiKeys[connection.id] ?? ""}
              activeId={activeId}
              run={run}
            />
          ))}
          <div className="w-72 pt-1">
            <Select
              value=""
              options={templateOptions}
              placeholder={t("settings.llm.connections.add")}
              isClearable={false}
              onChange={(value) => {
                if (value) void addConnection(value);
              }}
            />
          </div>
        </div>
      </SettingsGroup>
    </div>
  );
};

interface RowProps {
  connection: LlmConnection;
  template: PostProcessProvider | undefined;
  models: LlmModelConfig[];
  apiKey: string;
  activeId: string | null;
  run: (
    call: () => Promise<{ status: string; error?: unknown }>,
  ) => Promise<boolean>;
}

/// Eine Verbindung: Kopfzeile mit Name, Vorlage und Schalter; aufgeklappt
/// die Einrichtung — Adresse, Schluessel, Modelle laden und freigeben.
const ConnectionRow: React.FC<RowProps> = ({
  connection,
  template,
  models,
  apiKey,
  activeId,
  run,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [remote, setRemote] = useState<string[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [remoteError, setRemoteError] = useState<string | null>(null);

  const enabled = connection.enabled !== false;
  const keyless = KEYLESS_KINDS.has(connection.kind);
  const released = new Set(models.map((m) => m.remote_id));

  const upsert = (patch: Partial<LlmConnection>) =>
    run(() => commands.llmUpsertConnection({ ...connection, ...patch }));

  const loadModels = async () => {
    setLoading(true);
    setRemoteError(null);
    const result = await commands.llmListRemoteModels(connection.id);
    setLoading(false);
    if (result.status === "ok") setRemote(result.data);
    else setRemoteError(String(result.error));
  };

  const release = (remoteId: string, on: boolean) =>
    run(() =>
      on
        ? commands.llmUpsertModel({
            id: "",
            connection_id: connection.id,
            remote_id: remoteId,
            label: remoteId,
            enabled: true,
            tags: [],
          })
        : commands.llmRemoveModel(`${connection.id}:${remoteId}`),
    );

  return (
    <div className="rounded-md border border-mid-gray/20">
      <div className="flex items-center gap-2 px-3 py-2">
        <button
          type="button"
          className="p-1 rounded hover:bg-mid-gray/15"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          aria-label={connection.label}
        >
          {open ? (
            <ChevronDown width={16} height={16} />
          ) : (
            <ChevronRight width={16} height={16} />
          )}
        </button>
        <span
          className={`text-sm font-medium ${enabled ? "" : "text-text/50"}`}
        >
          {connection.label}
        </span>
        <Badge variant="secondary">{template?.label ?? connection.kind}</Badge>
        {!enabled && (
          <Badge variant="secondary">{t("settings.llm.connections.off")}</Badge>
        )}
        {models.length > 0 && (
          <span className="text-xs text-text/50">
            {t("settings.llm.connections.releasedCount", {
              count: models.length,
            })}
          </span>
        )}
        <span className="flex-1" />
        {confirmRemove ? (
          <>
            <Button
              size="sm"
              variant="danger-ghost"
              onClick={() =>
                void run(() => commands.llmRemoveConnection(connection.id))
              }
            >
              {t("settings.llm.connections.removeConfirm")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => setConfirmRemove(false)}
            >
              {t("common.cancel")}
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            variant="danger-ghost"
            onClick={() => setConfirmRemove(true)}
            title={t("settings.llm.connections.remove")}
            aria-label={t("settings.llm.connections.remove")}
          >
            <Trash2 width={14} height={14} />
          </Button>
        )}
      </div>

      {open && (
        <div className="border-t border-mid-gray/20 px-3 py-3 space-y-3">
          <ToggleSwitch
            checked={enabled}
            onChange={(checked) => void upsert({ enabled: checked })}
            label={t("settings.llm.connections.enabled")}
            description={t("settings.llm.connections.enabledDescription")}
            descriptionMode="tooltip"
          />
          {template?.allow_base_url_edit && (
            <label className="block text-sm">
              <span className="text-text/70">
                {t("settings.llm.connections.baseUrl")}
              </span>
              <Input
                type="text"
                defaultValue={connection.base_url}
                onBlur={(e) => {
                  const value = e.target.value.trim();
                  if (value && value !== connection.base_url)
                    void upsert({ base_url: value });
                }}
                className="w-full mt-1"
              />
            </label>
          )}
          {!keyless && (
            <label className="block text-sm">
              <span className="text-text/70">
                {t("settings.llm.connections.apiKey")}
              </span>
              <ApiKeyField
                value={apiKey}
                onBlur={(value) =>
                  void run(() => commands.llmSetApiKey(connection.id, value))
                }
                disabled={false}
                className="w-full mt-1"
              />
            </label>
          )}

          {/* Monatsbudget: leer heisst keins. Ab 80 % warnt die Fussleiste;
              nur mit "hart" werden Aufrufe bei 100 % verweigert. */}
          <div className="flex flex-wrap items-end gap-3" data-budget-row>
            <label className="block text-sm">
              <span className="text-text/70">
                {t("settings.llm.connections.budget")}
              </span>
              <Input
                type="text"
                variant="compact"
                defaultValue={
                  connection.monthly_budget_usd == null
                    ? ""
                    : String(connection.monthly_budget_usd)
                }
                placeholder={t("settings.llm.connections.budgetNone")}
                onBlur={(e) => {
                  const raw = e.target.value.trim().replace(",", ".");
                  const next = raw === "" ? null : Number(raw);
                  if (next !== null && (Number.isNaN(next) || next < 0)) return;
                  if (next !== (connection.monthly_budget_usd ?? null))
                    void upsert({ monthly_budget_usd: next });
                }}
                className="w-32 mt-1"
                aria-label={t("settings.llm.connections.budget")}
              />
            </label>
            <ToggleSwitch
              checked={connection.budget_enforced === true}
              onChange={(checked) => void upsert({ budget_enforced: checked })}
              label={t("settings.llm.connections.budgetEnforced")}
              description={t(
                "settings.llm.connections.budgetEnforcedDescription",
              )}
              descriptionMode="tooltip"
              disabled={connection.monthly_budget_usd == null}
            />
          </div>

          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void loadModels()}
              disabled={loading}
            >
              <RefreshCw width={14} height={14} />
              {loading
                ? t("settings.llm.connections.loading")
                : t("settings.llm.connections.loadModels")}
            </Button>
            <span className="text-xs text-text/50">
              {t("settings.llm.connections.releaseHint")}
            </span>
          </div>
          {remoteError && (
            <p className="text-sm text-red-500 break-words">{remoteError}</p>
          )}
          {remote && remote.length === 0 && (
            <p className="text-sm text-text/60">
              {t("settings.llm.connections.remoteEmpty")}
            </p>
          )}
          {remote && remote.length > 0 && (
            <ul className="max-h-56 overflow-y-auto space-y-1 text-sm">
              {remote.map((id) => (
                <li key={id}>
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={released.has(id)}
                      onChange={(e) => void release(id, e.target.checked)}
                    />
                    <span className="truncate">{id}</span>
                  </label>
                </li>
              ))}
            </ul>
          )}

          {models.length > 0 && (
            <div className="space-y-2">
              <p className="text-xs uppercase tracking-wide text-text/50">
                {t("settings.llm.connections.released")}
              </p>
              {models.map((m) => (
                <ModelRow
                  key={m.id}
                  model={m}
                  isActive={activeId === m.id}
                  run={run}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

/// Ein freigegebenes Modell mit seinen Limits und Preisen. Die Zahlen sind
/// getrennt, weil "Tokenlimit" allein drei Dinge meinen kann; leer heisst
/// unbekannt, nicht null.
const ModelRow: React.FC<{
  model: LlmModelConfig;
  isActive: boolean;
  run: RowProps["run"];
}> = ({ model, isActive, run }) => {
  const { t } = useTranslation();
  const [details, setDetails] = useState(false);

  const num = (value: string): number | null => {
    const n = Number(value.replace(",", "."));
    return value.trim() === "" || Number.isNaN(n) ? null : n;
  };
  const save = (patch: Partial<LlmModelConfig>) =>
    run(() => commands.llmUpsertModel({ ...model, ...patch }));

  const field = (
    key:
      | "context_limit"
      | "max_output_tokens"
      | "price_input_per_mtok"
      | "price_output_per_mtok",
    label: string,
  ) => (
    <label className="block text-xs">
      <span className="text-text/60">{label}</span>
      <Input
        type="text"
        variant="compact"
        defaultValue={model[key] == null ? "" : String(model[key])}
        onBlur={(e) => {
          const next = num(e.target.value);
          if (next !== (model[key] ?? null)) void save({ [key]: next });
        }}
        className="w-full mt-0.5"
      />
    </label>
  );

  return (
    <div className="rounded border border-mid-gray/15 px-2 py-1.5">
      <div className="flex items-center gap-2 text-sm">
        <span className="font-medium truncate">{model.label}</span>
        {isActive && (
          <Badge variant="success">{t("settings.llm.model.active")}</Badge>
        )}
        <span className="flex-1" />
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setDetails(!details)}
          aria-expanded={details}
        >
          {t("settings.llm.model.details")}
        </Button>
        <Button
          size="sm"
          variant="danger-ghost"
          onClick={() => void run(() => commands.llmRemoveModel(model.id))}
          title={t("settings.llm.model.remove")}
          aria-label={t("settings.llm.model.remove")}
        >
          <Trash2 width={14} height={14} />
        </Button>
      </div>
      {details && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 mt-2">
          {field("context_limit", t("settings.llm.model.context"))}
          {field("max_output_tokens", t("settings.llm.model.maxOutput"))}
          {field("price_input_per_mtok", t("settings.llm.model.priceIn"))}
          {field("price_output_per_mtok", t("settings.llm.model.priceOut"))}
        </div>
      )}
    </div>
  );
};

export default LlmConnectionsSettings;
