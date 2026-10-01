import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, Lock } from "lucide-react";
import {
  commands,
  type AgentClientView,
  type BridgeStatus,
  type GrantMode,
  type NewAgentClient,
  type ToolView,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { GRANT_MODES, errorText } from "./model";

interface AgentClientsPanelProps {
  /** Die Agent-Integration, zu der die Zugänge gehören (ihre Rechte-Matrix ist die Obergrenze). */
  integrationId: string;
}

type Confirm = { id: string; kind: "revoke" | "delete" } | null;
type CopyState = "idle" | "copied" | "failed";

/** Zeitpunkt (ms UTC) in der Sprache der Oberfläche. */
const stamp = (ms: number, language: string): string =>
  new Date(ms).toLocaleString(language, {
    dateStyle: "medium",
    timeStyle: "short",
  });

/**
 * Zugänge für externe Agenten (A7): je Programm (Claude Code, Codex, ein Skript) ein
 * Zugang mit eigenem Schlüssel und einem Recht je Werkzeug. Der Schlüssel erscheint
 * genau einmal beim Anlegen; das Backend speichert nur seinen Fingerabdruck. Die
 * wirksamen Rechte rechnet das Backend (dieselbe Regel wie beim Aufruf): steht die
 * Obergrenze der Integration auf „aus“, zeigt die Oberfläche den Grund, statt etwas
 * anderes zu behaupten. „Aufnahme starten“ bietet „erlaubt“ nie an.
 */
export const AgentClientsPanel: React.FC<AgentClientsPanelProps> = ({
  integrationId,
}) => {
  const { t, i18n } = useTranslation();
  const [clients, setClients] = useState<AgentClientView[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [bridge, setBridge] = useState<BridgeStatus | null>(null);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<NewAgentClient | null>(null);
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const [confirm, setConfirm] = useState<Confirm>(null);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const reload = useCallback(async () => {
    try {
      const [list, status] = await Promise.all([
        commands.agentClientList(),
        commands.agentBridgeStatus(),
      ]);
      if (list.status === "ok") {
        setClients(list.data ?? []);
        setLoadFailed(false);
      } else {
        setLoadFailed(true);
      }
      setBridge(status);
    } catch {
      setLoadFailed(true);
    }
  }, []);

  useEffect(() => {
    void reload();
    return () => {
      if (resetTimer.current) clearTimeout(resetTimer.current);
    };
  }, [reload]);

  const fail = (raw: unknown) =>
    setError(errorText(raw) || t("integrations.errors.generic"));

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await commands.agentClientCreate(
        name.trim(),
        integrationId,
      );
      if (result.status === "ok") {
        setCreated(result.data);
        setCopyState("idle");
        setCreating(false);
        setName("");
        await reload();
      } else {
        fail(result.error);
      }
    } catch (e) {
      fail(e);
    }
    setBusy(false);
  };

  const copy = async () => {
    if (!created) return;
    let next: CopyState = "copied";
    try {
      await navigator.clipboard.writeText(created.token);
    } catch {
      next = "failed";
    }
    setCopyState(next);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopyState("idle"), 2500);
  };

  const act = async (
    action: () => Promise<
      { status: "ok" } | { status: "error"; error: string }
    >,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await action();
      if (result.status === "error") fail(result.error);
    } catch (e) {
      fail(e);
    }
    setConfirm(null);
    setBusy(false);
    await reload();
  };

  const setToolMode = async (
    clientId: string,
    tool: string,
    mode: GrantMode,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await commands.agentClientSetToolMode(
        clientId,
        tool,
        mode,
      );
      if (result.status === "ok") {
        const next = result.data;
        setClients((list) =>
          (list ?? []).map((c) => (c.client.id === clientId ? next : c)),
        );
      } else {
        fail(result.error);
      }
    } catch (e) {
      fail(e);
    }
    setBusy(false);
  };

  const mine = (clients ?? []).filter(
    (c) => c.client.integration_id === integrationId,
  );

  return (
    <section
      className="space-y-3"
      aria-labelledby="int-clients"
      data-testid="agent-clients"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 id="int-clients" className="text-sm font-semibold">
          {t("integrations.detail.clients")}
        </h3>
        {!creating && (
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() => {
              setCreating(true);
              setCreated(null);
              setError(null);
            }}
            data-testid="agent-client-create-open"
          >
            {t("integrations.agentClients.create.open")}
          </Button>
        )}
      </div>
      <p className="text-xs text-text-muted">
        {t("integrations.agentClients.intro")}
      </p>

      {bridge && (
        <div
          className={`rounded-lg px-3 py-2 text-xs ${
            bridge.running ? "bg-green-500/10" : "bg-amber-500/10"
          }`}
          role="status"
          data-testid="agent-bridge-status"
          data-running={bridge.running}
        >
          <p className="font-medium">
            {bridge.running
              ? t("integrations.agentClients.bridge.running")
              : t("integrations.agentClients.bridge.stopped")}
          </p>
          {bridge.pipe_name && (
            <p className="break-all text-text-muted">
              {t("integrations.agentClients.bridge.pipe", {
                name: bridge.pipe_name,
              })}
            </p>
          )}
          {bridge.error && (
            <p className="break-words" data-testid="agent-bridge-error">
              {t("integrations.agentClients.bridge.reason", {
                error: bridge.error,
              })}
            </p>
          )}
          {bridge.running && (
            <p className="text-text-muted">
              {t("integrations.agentClients.bridge.safe")}
            </p>
          )}
        </div>
      )}

      {error && (
        <p
          className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-red-400"
          role="alert"
          data-testid="agent-clients-error"
        >
          {error}
        </p>
      )}

      {creating && (
        <form
          className="space-y-2 rounded-lg border border-mid-gray/20 p-3"
          data-testid="agent-client-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim() && !busy) void create();
          }}
        >
          <label className="block text-sm font-medium" htmlFor="agent-name">
            {t("integrations.agentClients.create.name")}
          </label>
          <Input
            id="agent-name"
            value={name}
            maxLength={60}
            placeholder={t("integrations.agentClients.create.placeholder")}
            onChange={(e) => setName(e.target.value)}
            data-testid="agent-client-name"
            className="w-full max-w-sm"
            autoFocus
          />
          <div className="flex flex-wrap gap-2">
            <Button
              type="submit"
              size="sm"
              disabled={busy || !name.trim()}
              data-testid="agent-client-create-submit"
            >
              {t("integrations.agentClients.create.submit")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => {
                setCreating(false);
                setName("");
                setError(null);
              }}
              data-testid="agent-client-create-cancel"
            >
              {t("integrations.cancel")}
            </Button>
          </div>
        </form>
      )}

      {created && (
        <div
          className="space-y-2 rounded-lg border border-logo-primary/60 bg-logo-primary/10 p-3"
          role="region"
          aria-label={t("integrations.agentClients.token.title", {
            label: created.client.label,
          })}
          data-testid="agent-token-card"
        >
          <p className="text-sm font-medium">
            {t("integrations.agentClients.token.title", {
              label: created.client.label,
            })}
          </p>
          <p className="text-xs" data-testid="agent-token-warning">
            {t("integrations.agentClients.token.warning")}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <code
              className="min-w-0 max-w-full break-all rounded-md bg-mid-gray/20 px-2 py-1 font-mono text-xs"
              data-testid="agent-token-value"
            >
              {created.token}
            </code>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void copy()}
              data-testid="agent-token-copy"
            >
              {copyState === "copied" ? (
                <Check size={14} aria-hidden="true" />
              ) : (
                <Copy size={14} aria-hidden="true" />
              )}
              {copyState === "copied"
                ? t("integrations.agentClients.token.copied")
                : t("integrations.agentClients.token.copy")}
            </Button>
          </div>
          {copyState === "failed" && (
            <p
              className="text-xs text-status-amber"
              role="status"
              data-testid="agent-token-copy-failed"
            >
              {t("integrations.agentClients.token.copyFailed")}
            </p>
          )}
          <p className="text-xs text-text-muted">
            {t("integrations.agentClients.token.use")}
          </p>
          <Button
            size="sm"
            onClick={() => {
              setCreated(null);
              setCopyState("idle");
            }}
            data-testid="agent-token-done"
          >
            {t("integrations.agentClients.token.done")}
          </Button>
        </div>
      )}

      {loadFailed && (
        <p className="text-sm text-red-400" role="alert">
          {t("integrations.agentClients.loadError")}
        </p>
      )}
      {clients && mine.length === 0 && !loadFailed && (
        <p
          className="text-sm text-text-muted"
          data-testid="agent-clients-empty"
        >
          {t("integrations.agentClients.empty")}
        </p>
      )}

      <ul className="space-y-3">
        {mine.map((view) => {
          const c = view.client;
          const revoked = c.revoked_at != null;
          const isConfirming = confirm?.id === c.id;
          return (
            <li
              key={c.id}
              className="space-y-3 rounded-lg border border-mid-gray/20 p-3"
              data-testid="agent-client"
              data-client-id={c.id}
              data-revoked={revoked}
            >
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="break-words text-sm font-medium">
                    {c.label}
                    <span
                      className={`ms-2 rounded-full px-2 py-0.5 text-xs font-normal ${
                        revoked
                          ? "bg-mid-gray/20 text-text-muted"
                          : "bg-green-500/15 text-text"
                      }`}
                      data-testid="agent-client-status"
                    >
                      {revoked
                        ? t("integrations.agentClients.client.revoked")
                        : t("integrations.agentClients.client.active")}
                    </span>
                  </p>
                  <p className="text-xs text-text-muted">
                    {t("integrations.agentClients.client.created", {
                      time: stamp(c.created_at, i18n.language),
                    })}
                    {" · "}
                    <span data-testid="agent-client-last-used">
                      {c.last_used_at != null
                        ? t("integrations.agentClients.client.lastUsed", {
                            time: stamp(c.last_used_at, i18n.language),
                          })
                        : t("integrations.agentClients.client.neverUsed")}
                    </span>
                  </p>
                  {revoked && c.revoked_at != null && (
                    <p className="text-xs text-text-muted">
                      {t("integrations.agentClients.client.revokedAt", {
                        time: stamp(c.revoked_at, i18n.language),
                      })}
                    </p>
                  )}
                </div>
                {!isConfirming && (
                  <div className="flex flex-wrap gap-2">
                    {!revoked && (
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={busy}
                        onClick={() => setConfirm({ id: c.id, kind: "revoke" })}
                        data-testid="agent-client-revoke"
                      >
                        {t("integrations.agentClients.client.revoke")}
                      </Button>
                    )}
                    <Button
                      variant="danger-ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() => setConfirm({ id: c.id, kind: "delete" })}
                      data-testid="agent-client-delete"
                    >
                      {t("integrations.agentClients.client.delete")}
                    </Button>
                  </div>
                )}
              </div>

              {isConfirming && confirm && (
                <div
                  className="flex flex-wrap items-center gap-2"
                  data-testid="agent-client-confirm"
                  data-kind={confirm.kind}
                >
                  <span className="text-sm">
                    {t(
                      confirm.kind === "revoke"
                        ? "integrations.agentClients.client.revokeConfirm"
                        : "integrations.agentClients.client.deleteConfirm",
                      { label: c.label },
                    )}
                  </span>
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={busy}
                    onClick={() =>
                      void act(() =>
                        confirm.kind === "revoke"
                          ? commands.agentClientRevoke(c.id)
                          : commands.agentClientDelete(c.id),
                      )
                    }
                    data-testid="agent-client-confirm-yes"
                  >
                    {t(
                      confirm.kind === "revoke"
                        ? "integrations.agentClients.client.revokeYes"
                        : "integrations.agentClients.client.deleteYes",
                    )}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirm(null)}
                    data-testid="agent-client-confirm-no"
                  >
                    {t("integrations.cancel")}
                  </Button>
                </div>
              )}

              <details open={!revoked} data-testid="agent-client-tools">
                <summary className="cursor-pointer text-sm font-medium">
                  {t("integrations.agentClients.client.tools")}
                </summary>
                <p className="mb-2 mt-1 text-xs text-text-muted">
                  {revoked
                    ? t("integrations.agentClients.client.frozen")
                    : t("integrations.agentClients.client.toolsHint")}
                </p>
                <div className="space-y-2">
                  {view.tools.map((tool) => (
                    <ToolRow
                      key={tool.name}
                      clientId={c.id}
                      tool={tool}
                      disabled={busy || revoked}
                      onChange={(mode) =>
                        void setToolMode(c.id, tool.name, mode)
                      }
                    />
                  ))}
                </div>
                {view.tools.some((x) => !x.available) && (
                  <p
                    className="mt-2 text-xs text-text-muted"
                    data-testid="agent-tools-unavailable-note"
                  >
                    {t("integrations.agentClients.unavailable")}
                  </p>
                )}
              </details>
            </li>
          );
        })}
      </ul>
    </section>
  );
};

interface ToolRowProps {
  clientId: string;
  tool: ToolView;
  disabled: boolean;
  onChange: (mode: GrantMode) => void;
}

/** Ein Werkzeug eines Zugangs: Recht (aus / fragen / erlaubt) und, was davon wirkt. */
const ToolRow: React.FC<ToolRowProps> = ({
  clientId,
  tool,
  disabled,
  onChange,
}) => {
  const { t } = useTranslation();
  const base = `integrations.agentClients.tools.${tool.name}`;
  const titleId = `agent-tool-title-${clientId}-${tool.name}`;
  const title = t(`${base}.title`, { defaultValue: tool.title });
  const description = t(`${base}.description`, {
    defaultValue: tool.description,
  });
  const neverAllow = tool.capability === "recording.start";
  const differs =
    tool.client_mode !== "off" && tool.effective_mode !== tool.client_mode;
  return (
    <div
      className="flex min-w-0 flex-wrap items-start justify-between gap-x-4 gap-y-2 rounded-md border border-mid-gray/15 p-2"
      role="group"
      aria-labelledby={titleId}
      data-testid={`agent-tool-${clientId}-${tool.name}`}
      data-tool={tool.name}
    >
      <div className="min-w-0 flex-1 basis-60">
        <p id={titleId} className="text-sm font-medium">
          {title}
          {!tool.available && (
            <span
              className="ms-2 rounded-full bg-mid-gray/20 px-2 py-0.5 text-xs font-normal text-text-muted"
              data-testid="agent-tool-unavailable"
            >
              {t("integrations.agentClients.unavailableBadge")}
            </span>
          )}
        </p>
        <p className="text-xs text-text-muted">{description}</p>
        {neverAllow && (
          <p className="mt-1 flex items-start gap-1 text-xs text-text-muted">
            <Lock size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
            {t("integrations.agentClients.neverAllowShort")}
          </p>
        )}
        {differs && (
          <p
            className="mt-1 text-xs text-text-muted"
            data-testid="agent-tool-effective"
          >
            {t("integrations.matrix.effective", {
              mode: t(`integrations.modes.${tool.effective_mode}`),
            })}
          </p>
        )}
        {tool.client_mode !== "off" &&
          tool.effective_mode === "off" &&
          tool.off_reason && (
            <p
              className="mt-1 text-xs text-status-amber"
              data-testid="agent-tool-off-reason"
              data-reason={tool.off_reason}
            >
              {t(`integrations.agentClients.offReason.${tool.off_reason}`, {
                defaultValue: tool.off_reason,
              })}
            </p>
          )}
      </div>
      <div
        role="radiogroup"
        aria-labelledby={titleId}
        className="inline-flex max-w-full shrink-0 overflow-hidden rounded-lg border border-mid-gray/30"
      >
        {GRANT_MODES.map((mode) => {
          const selected = tool.client_mode === mode;
          const blocked = disabled || (neverAllow && mode === "allow");
          return (
            <button
              key={mode}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={blocked}
              title={
                neverAllow && mode === "allow"
                  ? t("integrations.matrix.neverAllow")
                  : undefined
              }
              data-testid={`agent-tool-${clientId}-${tool.name}-${mode}`}
              onClick={() => {
                if (!selected) onChange(mode);
              }}
              className={`min-h-9 min-w-[4.25rem] cursor-pointer px-2 py-1 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                selected
                  ? mode === "off"
                    ? "bg-mid-gray/30 text-text"
                    : "bg-logo-primary text-on-accent"
                  : "bg-transparent text-text hover:bg-mid-gray/15"
              }`}
            >
              {t(`integrations.modes.${mode}`)}
            </button>
          );
        })}
      </div>
    </div>
  );
};
