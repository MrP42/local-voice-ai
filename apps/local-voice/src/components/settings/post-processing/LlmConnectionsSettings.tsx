import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Select as FluentSelect } from "@fluentui/react-components";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open as selectProgram } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import {
  commands,
  type CliStatus,
  type CliInstallInfo,
  type InstallFehler,
  type InstallFortschritt,
  type LlmConnection,
  type LlmModelConfig,
  type LlmModelSeen,
  type PostProcessProvider,
} from "@/bindings";
import {
  ChevronDown,
  ChevronRight,
  RefreshCw,
  Trash2,
  Zap,
} from "lucide-react";
import { useSettings } from "../../../hooks/useSettings";
import { availabilityKey, useLlmAvailability } from "@/lib/llmAvailability";
import { modelLabel } from "@/lib/modelNames";
import {
  cliSperre,
  modellListenFehler,
  modellListenSchluessel,
  type CliSperre,
} from "@/lib/llmFehler";

/// Stufen fuer den Standard-Effort (alle Abo-Modelle kennen sie).
const DEFAULT_EFFORTS = ["low", "medium", "high", "xhigh", "max"];
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Select } from "../../ui/Select";
import { Input } from "../../ui/Input";
import { Button } from "../../ui/Button";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import Badge from "../../ui/Badge";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";
import { ModelBadges } from "../../compliance/ModelBadges";
import { FunctionModels } from "./FunctionModels";
import {
  notifyComplianceChanged,
  useComplianceStore,
} from "@/stores/complianceStore";

/// Vorlagen, bei denen ein Schluessel nichts zu suchen hat: lokal, ohne Konto.
const KEYLESS_KINDS = new Set([
  "ollama",
  "vllm",
  "local",
  "apple_intelligence",
  // Abo ueber die CLI: angemeldet wird in der CLI selbst, nie in der App.
  "claude_cli",
  "codex_cli",
]);

/// API-Vorlagen, zu denen es ein Abo gibt: ohne Schluessel auf die Abo-Vorlage verweisen.
const SUBSCRIPTION_OF: Record<string, string> = {
  anthropic: "claude_cli",
  openai: "codex_cli",
  openai_eu: "codex_cli",
};

/// Seite des Anbieters, auf der man einen API-Schluessel bekommt.
const API_KEY_PAGES: Record<string, string> = {
  gemini: "https://aistudio.google.com/apikey",
  anthropic: "https://platform.claude.com/settings/keys",
  openai: "https://platform.openai.com/api-keys",
  openai_eu: "https://platform.openai.com/api-keys",
};

/// Vorlagen, die das Standard-Regelwerk "EU" sperrt. Ohne Hinweis richtet man
/// die Verbindung ein und scheitert erst bei der Modellwahl.
const BLOCKED_UNDER_EU = new Set([
  "claude_cli",
  "codex_cli",
  "gemini",
  "anthropic",
  "openai",
]);

/// Zustand der CLI und der Weg zur Anmeldung: die Anmeldung selbst laeuft in
/// der CLI, die App oeffnet sie nur und liest danach "angemeldet ja/nein".
/// API-Vorlage als Ausweg, wenn das Netz den Abo-Installer sperrt.
const API_INSTEAD_OF: Record<string, string> = {
  claude_cli: "anthropic",
  codex_cli: "openai",
};

const CliSignIn: React.FC<{
  kind: string;
  /** Legt eine Verbindung der Vorlage an (Ausweg „mit API-Schlüssel“). */
  onAddConnection?: (kind: string) => void;
}> = ({ kind, onAddConnection }) => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const policyDisabled = getSetting("cli_download_disabled") ?? false;
  const [status, setStatus] = useState<CliStatus | null>(null);
  const [info, setInfo] = useState<CliInstallInfo | null>(null);
  const [installFailure, setInstallFailure] = useState<InstallFehler | null>(null);
  const [progress, setProgress] = useState<InstallFortschritt | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  // Netz sperrt den Installer (lva#654): Meldung statt Konsolenfenster.
  const [blocked, setBlocked] = useState<CliSperre | null>(null);
  const [checking, setChecking] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const latest = useRef(0);

  const refresh = useCallback(async () => {
    const request = ++latest.current;
    const found = await commands.llmCliInstallInfo(kind).catch(() => null);
    if (request !== latest.current) return;
    if (found?.status === "ok") {
      setInfo(found.data);
      setDownloading(found.data.downloading);
      if (found.data.downloading) {
        setStatus((current) => current ?? { installed: found.data.found, logged_in: null });
        return;
      }
    }
    const result = await commands.llmCliStatus(kind).catch(() => null);
    if (request !== latest.current) return;
    if (result?.status === "ok") {
      setStatus(result.data);
      if (result.data.logged_in === true) setWaiting(false);
    } else {
      setStatus({ installed: found?.status === "ok" && found.data.found, logged_in: null });
      setInstallFailure({ code: "blockiert" });
    }
  }, [kind]);

  useEffect(() => {
    void refresh();
  }, [refresh, policyDisabled]);

  useEffect(() => {
    let disposed = false;
    const subscription = listen<InstallFortschritt>("llm-cli-install-progress", ({ payload }) => {
      if (disposed || payload.kind !== kind) return;
      if (payload.phase === "fertig") {
        setDownloading(false);
        setProgress(null);
        setInstallFailure(payload.error);
        if (!payload.error) setWaiting(true);
        void refreshSettings();
        void refresh();
      } else {
        setDownloading(true);
        setProgress(payload);
      }
    });
    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [kind, refresh, refreshSettings]);

  const download = async () => {
    setInstallFailure(null);
    setDownloading(true);
    setProgress(null);
    try {
      const result = await commands.llmCliDownload(kind);
      if (result.status === "error") setInstallFailure(result.error);
      else setWaiting(true);
    } catch {
      setInstallFailure({ code: "blockiert" });
    } finally {
      setDownloading(false);
      setProgress(null);
      await refreshSettings();
      await refresh();
    }
  };

  const choose = async (remove = false) => {
    setInstallFailure(null);
    try {
      const path = remove ? null : await selectProgram({
        multiple: false, directory: false,
        filters: [{ name: kind === "claude_cli" ? "claude.exe" : "codex.exe", extensions: ["exe"] }],
      });
      if (!remove && typeof path !== "string") return;
      const result = await commands.llmCliSetPath(kind, typeof path === "string" ? path : null);
      if (result.status === "error") setInstallFailure(result.error);
      await refreshSettings();
      await refresh();
    } catch {
      setInstallFailure({ code: "pfad" });
    }
  };

  // Nur nach "Anmelden": jede Abfrage startet einen Prozess der CLI.
  useEffect(() => {
    if (!waiting) return;
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [waiting, refresh]);

  const start = async (open: typeof commands.llmCliLogin) => {
    setFailure(null);
    setBlocked(null);
    setChecking(true);
    let result: Awaited<ReturnType<typeof open>>;
    try {
      result = await open(kind);
    } catch (e) {
      setFailure(String(e));
      return;
    } finally {
      setChecking(false);
    }
    if (result.status === "ok") {
      setWaiting(true);
      return;
    }
    if (result.error === "cli_program_blocked") {
      setInstallFailure({ code: "blockiert" });
      return;
    }
    const sperre = cliSperre(result.error);
    if (sperre) setBlocked(sperre);
    else setFailure(result.error.replace(/^cli_\w+:\s*/, ""));
  };

  const apiKind = API_INSTEAD_OF[kind];
  const installControls = (
    <div className="space-y-2" data-cli-install={kind}>
      {info?.configured_path && <p className="text-xs break-all">{info.configured_path}</p>}
      {info?.configured_path_invalid && (
        <p role="status" className="text-xs">{t("settings.llm.cliStatus.install.pathInvalid")}</p>
      )}
      {(policyDisabled || info?.download_disabled) && (
        <p className="text-xs">{t("settings.llm.cliStatus.install.errors.richtlinie")}</p>
      )}
      {downloading && (
        <div role="status" aria-live="polite" className="text-xs">
          {t(`settings.llm.cliStatus.install.phases.${progress?.phase ?? "pruefen"}`)}
          <progress aria-label={t("settings.llm.cliStatus.install.progress")}
            max={progress?.total ?? undefined}
            value={progress?.total ? progress.received : undefined} />
        </div>
      )}
      {installFailure && (
        <p role="alert" className="text-xs text-red-500">
          {t(`settings.llm.cliStatus.install.errors.${installFailure.code}`, {
            ziel: installFailure.code === "netz_gesperrt" ? installFailure.ziel : "",
          })}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {!policyDisabled && info && !info.download_disabled && (
          <Button size="sm" variant="secondary" disabled={downloading || checking}
            onClick={() => void download()} data-testid="cli-download">
            {t("settings.llm.cliStatus.install.download")}
          </Button>
        )}
        {downloading && (
          <Button size="sm" variant="secondary" data-testid="cli-download-cancel"
            onClick={() => void commands.llmCliCancelDownload(kind).then((result) => {
              if (result.status === "error") setInstallFailure(result.error);
            }).catch(() => setInstallFailure({ code: "blockiert" }))}>
            {t("settings.llm.cliStatus.install.cancel")}
          </Button>
        )}
        <Button size="sm" variant="secondary" disabled={downloading || checking}
          onClick={() => void choose()} data-testid="cli-select-program">
          {t("settings.llm.cliStatus.install.select")}
        </Button>
        {info?.configured_path && (
          <Button size="sm" variant="secondary" disabled={downloading || checking}
            onClick={() => void choose(true)}>
            {t("settings.llm.cliStatus.install.remove")}
          </Button>
        )}
        <Button size="sm" variant="secondary" onClick={() => {
          void commands.llmCliSetup(kind).then((result) => {
            if (result.status === "error") setInstallFailure({ code: "blockiert" });
          }).catch(() => setInstallFailure({ code: "blockiert" }));
        }}>
          {t("settings.llm.cliStatus.install.docs")}
        </Button>
        {apiKind && onAddConnection && (
          <Button size="sm" variant="secondary" onClick={() => onAddConnection(apiKind)}>
            {t(`settings.llm.cliStatus.blocked.useApi.${kind}`)}
          </Button>
        )}
      </div>
    </div>
  );
  const blockedNotice = blocked && (
    <div
      className="space-y-2 rounded-md border border-mid-gray/20 p-3"
      role="alert"
      data-cli-blocked={blocked}
    >
      <p className="text-sm font-medium">
        {t(`settings.llm.cliStatus.blocked.title.${kind}`)}
      </p>
      <p className="text-xs text-text/80">
        {t(`settings.llm.cliStatus.blocked.reason.${blocked}`)}
      </p>
      <p className="text-xs text-text/80">
        {t(`settings.llm.cliStatus.blocked.instead.${kind}`)}
      </p>
      {apiKind && onAddConnection && (
        <Button
          variant="primary"
          size="sm"
          data-testid="cli-blocked-use-api"
          onClick={() => onAddConnection(apiKind)}
        >
          {t(`settings.llm.cliStatus.blocked.useApi.${kind}`)}
        </Button>
      )}
    </div>
  );

  if (!status) return null;
  if (!status.installed) {
    return (
      <div className="space-y-1" data-cli-status="missing">
        <p className="text-xs text-text/80">
          {t(`settings.llm.cliStatus.missing.${kind}`)}
        </p>
        <p className="text-xs text-text/70">
          {t("settings.llm.cliStatus.install.help")}
        </p>
        {installControls}
        {blockedNotice}
        {failure && <p className="text-xs text-red-500">{failure}</p>}
      </div>
    );
  }
  if (status.logged_in === true) {
    return (
      <div className="space-y-1" data-cli-status="logged-in">
        <p className="text-xs text-text/80">{t("settings.llm.cliStatus.loggedIn")}</p>
        {installControls}
      </div>
    );
  }
  return (
    <div className="space-y-1" data-cli-status="logged-out">
      <p className="text-xs text-text/80">
        {t(
          status.logged_in === false
            ? "settings.llm.cliStatus.loggedOut"
            : "settings.llm.cliStatus.unknown",
        )}
      </p>
      <Button
        variant="secondary"
        size="sm"
        disabled={downloading || checking}
        onClick={() => void start(commands.llmCliLogin)}
      >
        {t("settings.llm.cliStatus.login")}
      </Button>
      {installControls}
      {failure && <p className="text-xs text-red-500">{failure}</p>}
    </div>
  );
};

/// Fehler beim Laden der Modelle als Klartext, eine Meldung je HTTP-Status
/// (lva#654); die Meldung des Anbieters (ohne Schluessel) steht dahinter.
const remoteErrorText = (
  t: (key: string, options?: Record<string, unknown>) => string,
  raw: string,
): string => {
  const fehler = modellListenFehler(raw);
  if (fehler) {
    const text = t(
      `settings.llm.connections.http.${modellListenSchluessel(fehler.status)}`,
      { status: fehler.status },
    );
    return fehler.anbieter
      ? `${text} ${t("settings.llm.connections.http.provider", { message: fehler.anbieter })}`
      : text;
  }
  const status = raw.match(/\((40[13])[^)]*\)/)?.[1];
  if (status) return t("settings.llm.connections.keyRejected", { status });
  return raw;
};

/// Anbieter, bei denen sich das Training im Konto abschalten laesst -- dort
/// zaehlt die Bestaetigung fuer das Regelwerk.
const OPT_OUT_KINDS = new Set(["mistral", "claude_cli", "codex_cli"]);

/// Bedrock-Regionen (OpenAI-kompatibler Mantle-Endpunkt), EU zuerst.
const BEDROCK_REGIONS = [
  "eu-central-1",
  "eu-west-1",
  "eu-south-1",
  "eu-north-1",
  "us-east-1",
  "us-east-2",
  "us-west-2",
];
const bedrockUrl = (region: string) =>
  `https://bedrock-mantle.${region}.api.aws/v1`;
const bedrockRegion = (url: string) =>
  /bedrock-mantle\.([a-z0-9-]+)\.api\.aws/.exec(url)?.[1] ?? "us-east-1";

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
  // Abo-Verbindungen ohne installierte CLI laufen hier nicht; die App nutzt
  // dann den Ersatz (lokales Modell zuerst).
  const { unavailable_connection_ids: missing, fallback_model_id: fallbackId } =
    useLlmAvailability(
      availabilityKey(
        connections,
        models,
        activeId,
        getSetting("compliance_profile"),
      ),
    );
  const activeConnectionId =
    models.find((m) => m.id === activeId)?.connection_id ?? null;
  const activeMissing =
    activeConnectionId !== null && missing.includes(activeConnectionId);
  const fallbackModel = models.find((m) => m.id === fallbackId) ?? null;
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
      notifyComplianceChanged();
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
    const name = modelLabel(m.remote_id, m.label);
    return conn ? `${name} · ${conn.label}` : name;
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
            {activeMissing && (
              <p
                className="mt-1 text-xs text-orange-500"
                data-llm-active-fallback={fallbackModel?.id ?? ""}
              >
                {fallbackModel
                  ? t("settings.llm.unavailable.fallback", {
                      model: modelLabel(
                        fallbackModel.remote_id,
                        fallbackModel.label,
                      ),
                    })
                  : t("settings.llm.unavailable.noFallback")}
              </p>
            )}
          </div>
        </SettingContainer>
        <SettingContainer
          title={t("settings.llm.defaultEffort.title")}
          description={t("settings.llm.defaultEffort.description")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <FluentSelect
            value={(getSetting("llm_default_effort") ?? "medium") as string}
            onChange={(e) =>
              void run(() => commands.llmSetDefaultEffort(e.target.value))
            }
            className="w-72"
            data-testid="default-effort"
            aria-label={t("settings.llm.defaultEffort.title")}
          >
            {DEFAULT_EFFORTS.map((level) => (
              <option key={level} value={level}>
                {t(`settings.llm.effortLevels.${level}`)}
              </option>
            ))}
          </FluentSelect>
        </SettingContainer>
        <SettingContainer
          title={t("settings.llm.autoUpdate.title")}
          description={t("settings.llm.autoUpdate.description")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <FluentSelect
            value={(getSetting("llm_auto_update_models") ?? "ask") as string}
            onChange={(e) =>
              void run(async () => {
                const result = await commands.llmSetAutoUpdateModels(
                  e.target.value,
                );
                // Auf "automatisch" gestellt: gleich nachsehen, was schon wartet.
                if (result.status === "ok" && e.target.value === "on")
                  await commands.llmCheckNewModels();
                return result;
              })
            }
            className="w-72"
            data-testid="auto-update-models"
            aria-label={t("settings.llm.autoUpdate.title")}
          >
            {["ask", "on", "off"].map((mode) => (
              <option key={mode} value={mode}>
                {t(`settings.llm.autoUpdate.modes.${mode}`)}
              </option>
            ))}
          </FluentSelect>
        </SettingContainer>
        <ToggleSwitch
          checked={getSetting("cli_download_disabled") ?? false}
          onChange={(checked) =>
            void run(async () => {
              const result = await commands.llmCliSetDownloadPolicy(checked);
              return result.status === "ok" ? result : {
                status: "error" as const,
                error: t(`settings.llm.cliStatus.install.errors.${result.error.code}`),
              };
            })
          }
          testId="cli-download-disabled"
          label={t("settings.llm.cliStatus.install.policy")}
          description={t("settings.llm.cliStatus.install.policyHelp")}
          descriptionMode="tooltip"
          grouped={true}
        />
      </SettingsGroup>

      {selectable.length > 0 && (
        <FunctionModels selectable={selectable} labelFor={labelFor} run={run} />
      )}

      <SettingsGroup
        title={t("settings.llm.connections.title")}
        anchor="connections"
      >
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
              unavailable={missing.includes(connection.id)}
              run={run}
              onAddConnection={(kind) => void addConnection(kind)}
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
  /** Abo-Verbindung, deren CLI auf diesem Rechner fehlt. */
  unavailable: boolean;
  run: (
    call: () => Promise<{ status: string; error?: unknown }>,
  ) => Promise<boolean>;
  /** Legt eine neue Verbindung der Vorlage an. */
  onAddConnection: (kind: string) => void;
}

/// Eine Verbindung: Kopfzeile mit Name, Vorlage und Schalter; aufgeklappt
/// die Einrichtung — Adresse, Schluessel, Modelle laden und freigeben.
const ConnectionRow: React.FC<RowProps> = ({
  connection,
  template,
  models,
  apiKey,
  activeId,
  unavailable,
  run,
  onAddConnection,
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
  const { getSetting } = useSettings();
  const complianceProfile = getSetting("compliance_profile") ?? "eu";
  // Vom Anbieter neu angeboten, aber noch nicht freigegeben (Pruefung beim Start).
  const history = (getSetting("llm_model_history") ?? []) as LlmModelSeen[];
  const isNew = (remoteId: string) =>
    history.some(
      (h) => h.key === `${connection.id}:${remoteId}` && h.status === "new",
    );

  const upsert = (patch: Partial<LlmConnection>) =>
    run(() => commands.llmUpsertConnection({ ...connection, ...patch }));

  const loadModels = async () => {
    setRemoteError(null);
    // Ohne Schluessel gar nicht erst fragen: der Anbieter antwortet nur mit 401.
    if (!keyless && !apiKey.trim()) {
      const abo = SUBSCRIPTION_OF[connection.kind];
      setRemoteError(
        abo
          ? t("settings.llm.connections.keyMissingAbo", {
              abo: t(`settings.llm.connections.aboTemplate.${abo}`),
            })
          : t("settings.llm.connections.keyMissing"),
      );
      return;
    }
    setLoading(true);
    const result = await commands.llmListRemoteModels(connection.id);
    setLoading(false);
    if (result.status === "ok") setRemote(result.data);
    else setRemoteError(remoteErrorText(t, String(result.error)));
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
        {enabled && unavailable && (
          <span data-connection-unavailable={connection.id}>
            <Badge variant="warning">
              {t("settings.llm.unavailable.short")}
            </Badge>
          </span>
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
          {(connection.kind === "claude_cli" ||
            connection.kind === "codex_cli") && (
            <p className="text-xs text-text/60" data-cli-hint={connection.kind}>
              {t(`settings.llm.cli.${connection.kind}`)}
            </p>
          )}
          {(connection.kind === "claude_cli" ||
            connection.kind === "codex_cli") && (
            <CliSignIn
              kind={connection.kind}
              onAddConnection={onAddConnection}
            />
          )}
          {complianceProfile === "eu" &&
            BLOCKED_UNDER_EU.has(connection.kind) && (
              <p
                className="text-xs text-text/80"
                data-eu-hint={connection.kind}
              >
                {t("settings.llm.connections.blockedUnderEu")}
              </p>
            )}
          {connection.kind === "bedrock_mantle" && (
            <label className="block text-sm">
              <span className="text-text/70">
                {t("settings.llm.connections.region")}
              </span>
              <div className="mt-1 w-72">
                <Select
                  value={bedrockRegion(connection.base_url)}
                  options={BEDROCK_REGIONS.map((r) => ({
                    value: r,
                    label: t(`settings.llm.regions.${r}`, { defaultValue: r }),
                  }))}
                  isClearable={false}
                  onChange={(value) => {
                    if (value) void upsert({ base_url: bedrockUrl(value) });
                  }}
                />
              </div>
            </label>
          )}
          {OPT_OUT_KINDS.has(connection.kind) && (
            <ToggleSwitch
              checked={connection.training_opt_out === true}
              onChange={(checked) => void upsert({ training_opt_out: checked })}
              label={t("settings.llm.connections.trainingOptOut")}
              description={t(
                "settings.llm.connections.trainingOptOutDescription",
              )}
              descriptionMode="inline"
            />
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
              {API_KEY_PAGES[connection.kind] && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="mt-1"
                  data-api-key-page={connection.kind}
                  onClick={() => void openUrl(API_KEY_PAGES[connection.kind])}
                >
                  {t("settings.llm.connections.getApiKey")}
                </Button>
              )}
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
                    {!released.has(id) && isNew(id) && (
                      <span data-new-model={id}>
                        <Badge variant="success">
                          {t("settings.llm.connections.newBadge")}
                        </Badge>
                      </span>
                    )}
                  </label>
                </li>
              ))}
            </ul>
          )}

          {models.length > 0 && (
            <div className="space-y-2">
              <p className="text-xs text-text/50">
                {t("settings.llm.connections.released")}
              </p>
              {models.map((m) => (
                <ModelRow
                  key={m.id}
                  model={m}
                  isActive={activeId === m.id}
                  unavailable={unavailable}
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
  unavailable: boolean;
  run: RowProps["run"];
}> = ({ model, isActive, unavailable, run }) => {
  const { t } = useTranslation();
  const assessment = useComplianceStore((s) => s.byModel[model.id]);
  const { getSetting } = useSettings();
  const defaultEffort = (getSetting("llm_default_effort") ??
    "medium") as string;
  const [details, setDetails] = useState(false);
  // Effort-Stufen (nur Abo-Modelle ueber die CLI kennen welche).
  const [efforts, setEfforts] = useState<string[]>([]);
  // Fast-Modus (nur Codex-Modelle, die ihn laut Katalog anbieten).
  const [offersFast, setOffersFast] = useState(false);
  useEffect(() => {
    let cancelled = false;
    commands
      .llmModelEfforts(model.connection_id, model.remote_id)
      .then((list) => {
        if (!cancelled) setEfforts(list ?? []);
      })
      .catch(() => {
        // Ohne Backend: keine Auswahl.
      });
    commands
      .llmModelOffersFast(model.connection_id, model.remote_id)
      .then((offers) => {
        if (!cancelled) setOffersFast(offers === true);
      })
      .catch(() => {
        // Ohne Backend: keine Auswahl.
      });
    return () => {
      cancelled = true;
    };
  }, [model.connection_id, model.remote_id]);

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
        <span className="font-medium truncate" title={model.remote_id}>
          {modelLabel(model.remote_id, model.label)}
        </span>
        <ModelBadges assessment={assessment} />
        {isActive && (
          <span
            title={
              unavailable ? t("settings.llm.unavailable.activeHint") : undefined
            }
          >
            <Badge variant={unavailable ? "warning" : "success"}>
              {unavailable
                ? t("settings.llm.unavailable.activeBadge")
                : t("settings.llm.model.active")}
            </Badge>
          </span>
        )}
        <span className="flex-1" />
        {efforts.length > 0 && (
          <label className="flex items-center gap-1 text-xs text-text/60">
            {t("settings.llm.model.effort")}
            <FluentSelect
              value={model.effort ?? ""}
              onChange={(e) => void save({ effort: e.target.value || null })}
              size="small"
              data-effort-select={model.id}
            >
              <option value="">
                {t("settings.llm.model.effortDefault", {
                  level: t(`settings.llm.effortLevels.${defaultEffort}`, {
                    defaultValue: defaultEffort,
                  }),
                })}
              </option>
              {efforts.map((level) => (
                <option key={level} value={level}>
                  {t(`settings.llm.effortLevels.${level}`, {
                    defaultValue: level,
                  })}
                </option>
              ))}
            </FluentSelect>
          </label>
        )}
        {offersFast && (
          <button
            type="button"
            onClick={() => void save({ fast: !model.fast })}
            aria-pressed={!!model.fast}
            title={t(
              model.fast
                ? "settings.llm.model.fastOn"
                : "settings.llm.model.fastOff",
            )}
            aria-label={t("settings.llm.model.fast")}
            className={`rounded p-1 transition-colors ${
              model.fast
                ? "bg-yellow-400/20 text-yellow-500"
                : "text-text/40 hover:bg-mid-gray/10 hover:text-text/70"
            }`}
            data-fast-toggle={model.id}
          >
            <Zap
              className="h-4 w-4"
              fill={model.fast ? "currentColor" : "none"}
            />
          </button>
        )}
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
