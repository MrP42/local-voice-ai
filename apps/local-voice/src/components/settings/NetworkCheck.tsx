import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { AlertTriangle, CheckCircle2, Copy, XCircle } from "lucide-react";
import {
  commands,
  type NetzRow,
  type NetzState,
  type NetzcheckResult,
  type ProxyInfo,
} from "@/bindings";
import { SettingContainer } from "../ui/SettingContainer";
import { Button } from "../ui/Button";

const STATE_ICON: Record<NetzState, React.ReactNode> = {
  reachable: <CheckCircle2 className="h-4 w-4 text-green-600" aria-hidden />,
  limited: <AlertTriangle className="h-4 w-4 text-amber-600" aria-hidden />,
  unreachable: <XCircle className="h-4 w-4 text-red-600" aria-hidden />,
};

const formatMs = (value: number | null, none: string) =>
  value === null ? none : `${value} ms`;

const hostPort = (row: NetzRow) =>
  row.host.includes(":")
    ? `[${row.host}]:${row.port}`
    : `${row.host}:${row.port}`;

/**
 * Netzwerkprüfung (lva#496): Auf Knopfdruck prüft die App, ob sie die
 * eingerichteten Gegenstellen erreicht, und liefert einen Bericht für die IT
 * zum Kopieren. Status steht als Wort UND Symbol da, nie nur als Farbe.
 */
export const NetworkCheck: React.FC = () => {
  const { t } = useTranslation();
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<NetzcheckResult | null>(null);
  const [failed, setFailed] = useState(false);

  const run = async () => {
    setRunning(true);
    setFailed(false);
    try {
      const r = await commands.netzcheckRun();
      if (r.status === "ok") {
        setResult(r.data);
      } else {
        setFailed(true);
        toast.error(
          r.error === "busy"
            ? t("settings.app.networkCheck.busy")
            : t("settings.app.networkCheck.error"),
        );
      }
    } catch {
      setFailed(true);
      toast.error(t("settings.app.networkCheck.error"));
    } finally {
      setRunning(false);
    }
  };

  const copy = async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(result.report);
      toast.success(t("settings.app.networkCheck.report.copied"));
    } catch {
      toast.error(t("settings.app.networkCheck.report.copyFailed"));
    }
  };

  const counts = (rows: NetzRow[]) => ({
    ok: rows.filter((r) => r.state === "reachable").length,
    limited: rows.filter((r) => r.state === "limited").length,
    down: rows.filter((r) => r.state === "unreachable").length,
  });

  const proxyText = (proxy: ProxyInfo) => {
    const parts = proxy.entries.map((e) =>
      t("settings.app.networkCheck.proxy.via", {
        endpoint: e.endpoint,
        scope: t(`settings.app.networkCheck.proxy.scope.${e.scope}`, {
          defaultValue: e.scope,
        }),
      }),
    );
    if (proxy.pac) parts.push(t("settings.app.networkCheck.proxy.pac"));
    return parts.length === 0
      ? t("settings.app.networkCheck.proxy.none")
      : t("settings.app.networkCheck.proxy.detected", {
          list: parts.join("; "),
        });
  };

  const none = t("settings.app.networkCheck.measure.none");

  return (
    <div data-testid="network-check">
      <SettingContainer
        title={t("settings.app.networkCheck.title")}
        description={t("settings.app.networkCheck.description")}
        descriptionMode="inline"
        grouped={true}
      >
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void run()}
          disabled={running}
          aria-busy={running}
          data-testid="network-check-run"
        >
          {running
            ? t("settings.app.networkCheck.running")
            : t("settings.app.networkCheck.run")}
        </Button>
      </SettingContainer>

      <div
        role="status"
        aria-live="polite"
        className="px-4 pb-2 text-sm"
        data-testid="network-check-status"
      >
        {running && t("settings.app.networkCheck.running")}
        {!running && failed && t("settings.app.networkCheck.error")}
        {!running &&
          !failed &&
          result &&
          result.rows.length === 0 &&
          t("settings.app.networkCheck.none")}
        {!running &&
          !failed &&
          result &&
          result.rows.length > 0 &&
          t("settings.app.networkCheck.summary", counts(result.rows))}
      </div>

      {result && (
        <div className="px-4 pb-3 space-y-3">
          <p
            className="text-xs text-mid-gray"
            data-testid="network-check-proxy"
          >
            {proxyText(result.proxy)}
          </p>

          {result.rows.length > 0 && (
            <ul
              aria-label={t("settings.app.networkCheck.results")}
              className="space-y-2"
              data-testid="network-check-rows"
            >
              {result.rows.map((row) => (
                <li
                  key={`${row.category}-${row.host}-${row.port}`}
                  className="rounded-md border border-mid-gray/20 p-2 text-sm"
                  data-testid="network-check-row"
                  data-state={row.state}
                >
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    {STATE_ICON[row.state]}
                    <span className="font-medium">
                      {t(`settings.app.networkCheck.state.${row.state}`)}
                    </span>
                    <span className="text-mid-gray" aria-hidden>
                      ·
                    </span>
                    <span>
                      {t(`settings.app.networkCheck.category.${row.category}`)}
                    </span>
                    <span className="font-mono text-xs break-all">
                      {hostPort(row)}
                    </span>
                  </div>
                  <p className="mt-1">
                    {t(
                      `settings.app.networkCheck.impact.${row.category}.${row.state}`,
                    )}
                  </p>
                  {row.reason && (
                    <p className="mt-1 text-xs text-mid-gray">
                      {t(`settings.app.networkCheck.reason.${row.reason}`)}
                    </p>
                  )}
                  {row.issuer && (
                    <p className="mt-1 text-xs text-mid-gray">
                      {t("settings.app.networkCheck.issuer", {
                        issuer: row.issuer,
                      })}
                    </p>
                  )}
                  <p className="mt-1 text-xs text-mid-gray">
                    {t("settings.app.networkCheck.measure.dns")}:{" "}
                    {formatMs(row.dns_ms, none)} ·{" "}
                    {t("settings.app.networkCheck.measure.tls")}:{" "}
                    {formatMs(row.tls_ms, none)} ·{" "}
                    {t("settings.app.networkCheck.measure.http")}:{" "}
                    {row.http_status === null ? none : row.http_status} ·{" "}
                    {t("settings.app.networkCheck.measure.total")}:{" "}
                    {formatMs(row.total_ms, none)}
                  </p>
                </li>
              ))}
            </ul>
          )}

          {result.proxy.entries.length > 0 || result.proxy.pac ? (
            <p className="text-xs text-mid-gray">
              {t("settings.app.networkCheck.proxy.note")}
            </p>
          ) : null}

          <div className="space-y-2">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="secondary"
                size="sm"
                icon={<Copy className="h-4 w-4" aria-hidden />}
                onClick={() => void copy()}
                data-testid="network-check-copy"
              >
                {t("settings.app.networkCheck.report.copy")}
              </Button>
              <span className="text-xs text-mid-gray">
                {t("settings.app.networkCheck.report.privacy")}
              </span>
            </div>
            <details>
              <summary className="cursor-pointer text-sm">
                {t("settings.app.networkCheck.report.show")}
              </summary>
              <pre
                className="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 p-2 text-xs"
                tabIndex={0}
                aria-label={t("settings.app.networkCheck.report.title")}
                data-testid="network-check-report"
              >
                {result.report}
              </pre>
            </details>
          </div>
        </div>
      )}
    </div>
  );
};
