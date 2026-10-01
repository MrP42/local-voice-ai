import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react";
import {
  commands,
  type Caller,
  type Capability,
  type Direction,
  type GrantMode,
  type IntegrationView,
  type TestResult,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { RightsMatrix } from "./RightsMatrix";
import { TargetActions } from "./TargetActions";
import { TargetDialog } from "./TargetDialog";
import {
  DIRECTIONS,
  configText,
  errorText,
  folderPathOf,
  isTargetKind,
} from "./model";
import { KindIcon } from "./KindIcon";
import { AgentClientsPanel } from "./AgentClientsPanel";
import { M365Panel } from "./M365Panel";

interface IntegrationDetailProps {
  view: IntegrationView;
  onBack: () => void;
  onChanged: (view: IntegrationView) => void;
  onRemoved: (id: string) => void;
  onShowAudit: (id: string) => void;
}

/** Zeitpunkt (ms UTC) in der Sprache der Oberflaeche. */
const stamp = (ms: number, language: string): string =>
  new Date(ms).toLocaleString(language, {
    dateStyle: "medium",
    timeStyle: "short",
  });

/**
 * Detail einer Integration: Name, Schalter, Richtung, Rechte-Matrix, Test,
 * Protokoll-Link und Entfernen. Jede Aenderung geht sofort ans Backend und
 * ersetzt die Zeile mit dem, was das Backend zurueckgibt (die wirksamen Rechte
 * rechnet das Backend, nie die Oberflaeche).
 */
export const IntegrationDetail: React.FC<IntegrationDetailProps> = ({
  view,
  onBack,
  onChanged,
  onRemoved,
  onShowAudit,
}) => {
  const { t, i18n } = useTranslation();
  const { integration } = view;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState(integration.label);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);
  const [removing, setRemoving] = useState(false);
  const [editingSettings, setEditingSettings] = useState(false);

  const explain = (raw: string) => {
    const key = `integrations.errors.${raw}`;
    const text = t(key, { defaultValue: "" });
    return text || raw || t("integrations.errors.generic");
  };

  const run = async (action: () => Promise<IntegrationView | null>) => {
    setBusy(true);
    setError(null);
    try {
      const next = await action();
      if (next) onChanged(next);
    } catch (e) {
      setError(explain(errorText(e)));
    }
    setBusy(false);
  };

  const call = async <T,>(
    promise: Promise<
      { status: "ok"; data: T } | { status: "error"; error: string }
    >,
  ): Promise<T> => {
    const result = await promise;
    if (result.status === "error") throw result.error;
    return result.data;
  };

  const update = (patch: {
    label?: string;
    enabled?: boolean;
    direction?: Direction;
  }) =>
    run(() =>
      call(
        commands.integrationUpdate(
          integration.id,
          patch.label ?? null,
          patch.enabled ?? null,
          patch.direction ?? null,
        ),
      ),
    );

  const setGrant = (
    capability: Capability,
    caller: Caller,
    mode: GrantMode | null,
  ) =>
    run(() =>
      call(
        commands.integrationSetGrant(integration.id, capability, caller, mode),
      ),
    );

  const resetGrants = async () => {
    setBusy(true);
    setError(null);
    try {
      let last: IntegrationView | null = null;
      for (const cap of view.capabilities) {
        for (const m of cap.modes) {
          if (m.stored === null) continue;
          last = await call(
            commands.integrationSetGrant(
              integration.id,
              cap.capability,
              m.caller,
              null,
            ),
          );
        }
      }
      if (last) onChanged(last);
    } catch (e) {
      setError(explain(errorText(e)));
    }
    setBusy(false);
  };

  /** Das Backend hat `last_ok_at`/`last_error` gesetzt (Test, Kontoaktion): Zeile neu holen. */
  const refresh = async () => {
    const list = await commands.integrationsList();
    if (list.status === "ok") {
      const fresh = list.data?.find((v) => v.integration.id === integration.id);
      if (fresh) onChanged(fresh);
    }
  };

  const test = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      setTestResult(await call(commands.integrationTest(integration.id)));
      await refresh();
    } catch (e) {
      setError(explain(errorText(e)));
    }
    setTesting(false);
  };

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      await call(commands.integrationDelete(integration.id));
      onRemoved(integration.id);
    } catch (e) {
      setError(explain(errorText(e)));
      setBusy(false);
    }
  };

  const path = folderPathOf(integration.config_json);
  const cfg = (key: string) => configText(integration.config_json, key);
  const settingRows: { label: string; value: string }[] = (() => {
    const row = (label: string, value: string) =>
      value ? [{ label, value }] : [];
    switch (integration.kind) {
      case "smtp":
        return [
          ...row(
            t("integrations.detail.settingHost"),
            `${cfg("host")}:${cfg("port")} (${t(`integrations.target.smtp.${cfg("security") === "tls" ? "tls" : "starttls"}`)})`,
          ),
          ...row(t("integrations.detail.settingFrom"), cfg("from_address")),
          ...row(t("integrations.detail.settingLogin"), cfg("username")),
        ];
      case "obsidian":
        return [
          ...row(t("integrations.detail.settingSubfolder"), cfg("subfolder")),
          ...row(
            t("integrations.detail.settingArea"),
            cfg("context_area")
              ? t(`integrations.areas.${cfg("context_area")}`, {
                  defaultValue: cfg("context_area"),
                })
              : "",
          ),
        ];
      case "wissen":
        return [
          ...row(t("integrations.detail.settingEndpoint"), cfg("endpoint")),
          ...row(t("integrations.detail.settingTool"), cfg("search_tool")),
          ...row(t("integrations.detail.settingArea"), cfg("area")),
        ];
      case "webhook":
        // Nur der Server; die Adresse selbst ist ein Geheimnis und kommt nie zurueck.
        return row(t("integrations.detail.settingHost"), cfg("host"));
      case "folder":
        return row(t("integrations.detail.settingSubfolder"), cfg("subfolder"));
      default:
        return [];
    }
  })();
  const hasStored = view.capabilities.some((c) =>
    c.modes.some((m) => m.stored !== null),
  );
  const managed = view.calendar_managed;
  const nameChanged = name.trim() !== integration.label && name.trim() !== "";

  return (
    <div className="space-y-5" data-testid="integration-detail">
      <div>
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          data-testid="integrations-back"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          {t("integrations.back")}
        </Button>
      </div>

      <header className="flex items-start gap-3">
        <KindIcon kind={integration.kind} size={28} />
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-semibold break-words">
            {integration.label}
          </h2>
          <p className="text-sm text-text-muted">
            {t(`integrations.kinds.${integration.kind}`)}
            {path && <span className="break-all"> · {path}</span>}
          </p>
        </div>
        {!managed && (
          <Button
            variant={integration.enabled ? "secondary" : "primary"}
            size="sm"
            disabled={busy}
            onClick={() => void update({ enabled: !integration.enabled })}
            aria-pressed={integration.enabled}
            data-testid="enabled-toggle"
          >
            {integration.enabled
              ? t("integrations.detail.switchOff")
              : t("integrations.detail.switchOn")}
          </Button>
        )}
      </header>

      {!integration.enabled && (
        <p
          className="rounded-lg bg-amber-500/10 px-3 py-2 text-sm text-status-amber"
          role="status"
        >
          {t("integrations.matrix.blocked.integration_disabled")}
        </p>
      )}
      {error && (
        <p
          className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-status-red"
          role="alert"
          data-testid="integration-error"
        >
          {error}
        </p>
      )}

      <section className="space-y-2" aria-labelledby="int-conn">
        <h3 id="int-conn" className="text-sm font-semibold">
          {t("integrations.detail.connection")}
        </h3>
        {managed ? (
          <p className="text-sm text-text-muted">
            {t("integrations.detail.managedByCalendar")}
          </p>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            <label className="sr-only" htmlFor="int-name">
              {t("integrations.detail.name")}
            </label>
            <Input
              id="int-name"
              value={name}
              maxLength={120}
              onChange={(e) => setName(e.target.value)}
              className="min-w-0 flex-1 basis-48"
              data-testid="integration-name"
            />
            <Button
              variant="secondary"
              size="sm"
              disabled={busy || !nameChanged}
              onClick={() => void update({ label: name.trim() })}
              data-testid="integration-rename"
            >
              {t("integrations.detail.rename")}
            </Button>
          </div>
        )}
        <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[auto_1fr]">
          <dt className="text-text-muted">{t("integrations.detail.status")}</dt>
          <dd data-testid="integration-status">
            {integration.last_error ? (
              <span className="text-status-red break-words">
                {integration.last_error}
              </span>
            ) : integration.last_ok_at ? (
              t("integrations.detail.okAt", {
                time: stamp(integration.last_ok_at, i18n.language),
              })
            ) : (
              t("integrations.detail.notTested")
            )}
          </dd>
          {settingRows.map((row) => (
            <React.Fragment key={row.label}>
              <dt className="text-text-muted">{row.label}</dt>
              <dd className="break-all" data-testid="integration-setting">
                {row.value}
              </dd>
            </React.Fragment>
          ))}
          {view.secrets.map((s) => (
            <React.Fragment key={s.slot}>
              <dt className="text-text-muted">
                {t("integrations.detail.secret")}
              </dt>
              <dd>{t(`integrations.secret.${s.status}`)}</dd>
            </React.Fragment>
          ))}
        </dl>
        {isTargetKind(integration.kind) && (
          <div className="space-y-2">
            <div className="flex flex-wrap items-center gap-3">
              {integration.kind !== "webhook" && (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={testing}
                  onClick={() => void test()}
                  data-testid="integration-test"
                >
                  {testing
                    ? t("integrations.detail.testing")
                    : t("integrations.detail.test")}
                </Button>
              )}
              <Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onClick={() => setEditingSettings(true)}
                data-testid="integration-edit-settings"
              >
                {t("integrations.detail.editSettings")}
              </Button>
              {testResult && (
                <span
                  role="status"
                  data-testid="integration-test-result"
                  className={`text-sm ${testResult.ok ? "text-status-green" : "text-status-red"}`}
                >
                  {t(`integrations.test.${testResult.code}`, {
                    defaultValue: testResult.code,
                  })}
                </span>
              )}
            </div>
            {integration.kind === "webhook" && (
              <p
                className="text-xs text-text-muted"
                data-testid="webhook-no-test"
              >
                {t("integrations.detail.webhookNoTest")}
              </p>
            )}
            {testResult?.detail && (
              <p
                className="rounded-lg bg-red-500/10 px-3 py-2 text-sm break-words text-status-red"
                data-testid="integration-test-detail"
              >
                {testResult.detail}
              </p>
            )}
            <TargetActions view={view} onTouched={() => void refresh()} />
          </div>
        )}
      </section>

      {integration.kind === "m365" && (
        <M365Panel view={view} onChanged={refresh} />
      )}

      <section className="space-y-2" aria-labelledby="int-dir">
        <h3 id="int-dir" className="text-sm font-semibold">
          {t("integrations.detail.direction")}
        </h3>
        <p className="text-xs text-text-muted">
          {t("integrations.detail.directionHint")}
        </p>
        <div
          role="radiogroup"
          aria-labelledby="int-dir"
          className="inline-flex max-w-full flex-wrap overflow-hidden rounded-lg border border-mid-gray/30"
        >
          {DIRECTIONS.filter((d) => view.directions.includes(d)).map((d) => {
            const selected = integration.direction === d;
            const only = view.directions.length === 1;
            return (
              <button
                key={d}
                type="button"
                role="radio"
                aria-checked={selected}
                disabled={busy || only}
                data-testid={`direction-${d}`}
                onClick={() => {
                  if (!selected) void update({ direction: d });
                }}
                className={`min-h-9 cursor-pointer px-3 py-1 text-sm font-medium transition-colors disabled:cursor-not-allowed focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                  selected
                    ? "bg-logo-primary text-on-accent"
                    : "text-text hover:bg-mid-gray/15 disabled:opacity-50"
                }`}
              >
                {t(`integrations.directions.${d}`)}
              </button>
            );
          })}
        </div>
      </section>

      <section className="space-y-2" aria-labelledby="int-rights">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 id="int-rights" className="text-sm font-semibold">
            {t("integrations.detail.rights")}
          </h3>
          <Button
            variant="ghost"
            size="sm"
            disabled={busy || !hasStored}
            onClick={() => void resetGrants()}
            data-testid="grants-reset"
          >
            {t("integrations.detail.resetRights")}
          </Button>
        </div>
        <p className="text-xs text-text-muted">
          {t("integrations.detail.rightsHint")}
        </p>
        <RightsMatrix
          capabilities={view.capabilities}
          busy={busy}
          onChange={(cap, caller, mode) => void setGrant(cap, caller, mode)}
        />
      </section>

      {integration.kind === "agent" && (
        <AgentClientsPanel integrationId={integration.id} />
      )}

      <section className="flex flex-wrap items-center gap-3 border-t border-mid-gray/20 pt-4">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => onShowAudit(integration.id)}
          data-testid="integration-audit-link"
        >
          {t("integrations.detail.showAudit")}
        </Button>
        {!managed &&
          (removing ? (
            <div
              className="flex flex-wrap items-center gap-2"
              data-testid="integration-remove-confirm"
            >
              <span className="text-sm">
                {t("integrations.detail.removeConfirm", {
                  label: integration.label,
                })}
              </span>
              <Button
                variant="danger"
                size="sm"
                disabled={busy}
                onClick={() => void remove()}
                data-testid="integration-remove-yes"
              >
                {t("integrations.detail.removeYes")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setRemoving(false)}
              >
                {t("integrations.cancel")}
              </Button>
            </div>
          ) : (
            <Button
              variant="danger-ghost"
              size="sm"
              onClick={() => setRemoving(true)}
              data-testid="integration-remove"
            >
              {t("integrations.detail.remove")}
            </Button>
          ))}
      </section>

      {isTargetKind(integration.kind) && (
        <TargetDialog
          open={editingSettings}
          onOpenChange={setEditingSettings}
          kind={integration.kind}
          editing={view}
          onSaved={onChanged}
        />
      )}
    </div>
  );
};
