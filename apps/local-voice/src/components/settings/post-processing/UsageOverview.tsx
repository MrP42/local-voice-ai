import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type UsageBucket,
  type UsageRange,
  type UsageSummary,
} from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";

const RANGES: UsageRange[] = ["today", "week", "month", "all"];

/** Mikro-Dollar als Betrag mit Komma; unter einem Cent mit vier Stellen,
 *  damit ein Diktat für 0,0004 USD nicht als "0,00" verschwindet. */
export const formatUsd = (micro: number): string => {
  const usd = micro / 1_000_000;
  const digits = usd !== 0 && Math.abs(usd) < 0.01 ? 4 : 2;
  return usd.toFixed(digits).replace(".", ",");
};

/** Token mit Tausenderpunkt — 12.345, nicht 12345. */
export const formatTokens = (n: number): string => n.toLocaleString("de-DE");

/**
 * Was die Sprachmodelle verbraucht haben: Token und Kosten je Zeitraum,
 * aufgeschlüsselt nach Modell und Zweck, dazu die letzten Tage als Leiste.
 * Kosten sind, was zum Zeitpunkt des Aufrufs als Preis hinterlegt war — ein
 * lokales Modell ohne Preis kostet hier null, und das ist richtig so.
 */
export const UsageOverview: React.FC = () => {
  const { t } = useTranslation();
  const [range, setRange] = useState<UsageRange>("month");
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  const load = useCallback(async () => {
    try {
      const result = await commands.usageSummary(range);
      if (result.status === "ok") {
        setSummary(result.data);
        setError(null);
      } else {
        setError(String(result.error));
      }
    } catch {
      // Ohne Backend (Browser-Test) bleibt die Übersicht leer.
    }
  }, [range]);

  useEffect(() => {
    void load();
  }, [load]);

  const clear = async () => {
    setConfirmClear(false);
    await commands.usageClear();
    await load();
  };

  const purposeLabel = (key: string) =>
    t(`settings.llm.usage.purpose.${key}`, { defaultValue: key });

  const maxDay = Math.max(
    1,
    ...(summary?.by_day.map((d) => d.prompt_tokens + d.completion_tokens) ?? [
      0,
    ]),
  );

  return (
    <SettingsGroup title={t("settings.llm.usage.title")}>
      <div className="px-4 py-3 space-y-4" data-usage-overview>
        <div className="flex flex-wrap items-center gap-2">
          <div
            className="flex gap-1"
            role="tablist"
            aria-label={t("settings.llm.usage.range")}
          >
            {RANGES.map((r) => (
              <button
                key={r}
                type="button"
                role="tab"
                aria-selected={range === r}
                onClick={() => setRange(r)}
                className={`px-2.5 py-1 rounded text-xs transition-colors ${
                  range === r
                    ? "bg-logo-primary/15 text-logo-primary font-medium"
                    : "text-text/60 hover:bg-mid-gray/10"
                }`}
              >
                {t(`settings.llm.usage.ranges.${r}`)}
              </button>
            ))}
          </div>
          <span className="flex-1" />
          {confirmClear ? (
            <>
              <Button
                size="sm"
                variant="danger-ghost"
                onClick={() => void clear()}
              >
                {t("settings.llm.usage.clearConfirm")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onClick={() => setConfirmClear(false)}
              >
                {t("common.cancel")}
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="secondary"
              onClick={() => setConfirmClear(true)}
            >
              {t("settings.llm.usage.clear")}
            </Button>
          )}
        </div>

        {error && (
          <p className="text-sm text-red-500 break-words" role="alert">
            {error}
          </p>
        )}

        {summary && summary.calls === 0 && (
          <p className="text-sm text-text/60">
            {t("settings.llm.usage.empty")}
          </p>
        )}

        {summary && summary.calls > 0 && (
          <>
            <dl
              className="grid grid-cols-2 sm:grid-cols-4 gap-3"
              data-usage-totals
            >
              <Stat
                label={t("settings.llm.usage.calls")}
                value={formatTokens(summary.calls)}
                note={
                  summary.failed > 0
                    ? t("settings.llm.usage.failed", { count: summary.failed })
                    : undefined
                }
              />
              <Stat
                label={t("settings.llm.usage.promptTokens")}
                value={formatTokens(summary.prompt_tokens)}
              />
              <Stat
                label={t("settings.llm.usage.completionTokens")}
                value={formatTokens(summary.completion_tokens)}
              />
              <Stat
                label={t("settings.llm.usage.cost")}
                value={`${formatUsd(summary.cost_micro)} USD`}
              />
            </dl>

            {summary.by_day.length > 1 && (
              <div
                className="flex items-end gap-1 h-12"
                aria-label={t("settings.llm.usage.byDay")}
                data-usage-days
              >
                {summary.by_day.map((d) => {
                  const total = d.prompt_tokens + d.completion_tokens;
                  return (
                    <div
                      key={d.key}
                      className="flex-1 bg-logo-primary/40 rounded-sm min-w-1"
                      style={{
                        height: `${Math.max(4, (total / maxDay) * 100)}%`,
                      }}
                      title={`${d.key}: ${formatTokens(total)} Token, ${formatUsd(d.cost_micro)} USD`}
                    />
                  );
                })}
              </div>
            )}

            <div className="grid gap-4 md:grid-cols-2">
              <BucketTable
                title={t("settings.llm.usage.byModel")}
                rows={summary.by_model}
                label={(b) => b.label}
                testId="by-model"
              />
              <BucketTable
                title={t("settings.llm.usage.byPurpose")}
                rows={summary.by_purpose}
                label={(b) => purposeLabel(b.key)}
                testId="by-purpose"
              />
            </div>
            <p className="text-xs text-text/50">
              {t("settings.llm.usage.hint")}
            </p>
          </>
        )}
      </div>
    </SettingsGroup>
  );
};

const Stat: React.FC<{ label: string; value: string; note?: string }> = ({
  label,
  value,
  note,
}) => (
  <div>
    <dt className="text-xs text-text/50">{label}</dt>
    <dd className="text-base font-medium tabular-nums">{value}</dd>
    {note && <dd className="text-xs text-amber-600">{note}</dd>}
  </div>
);

const BucketTable: React.FC<{
  title: string;
  rows: UsageBucket[];
  label: (b: UsageBucket) => string;
  testId: string;
}> = ({ title, rows, label, testId }) => {
  const { t } = useTranslation();
  return (
    <div data-usage-table={testId}>
      <p className="text-xs uppercase tracking-wide text-text/50 mb-1">
        {title}
      </p>
      <table className="w-full text-sm">
        <thead className="text-xs text-text/50">
          <tr>
            <th className="text-start font-normal">
              {t("settings.llm.usage.colName")}
            </th>
            <th className="text-end font-normal">
              {t("settings.llm.usage.colCalls")}
            </th>
            <th className="text-end font-normal">
              {t("settings.llm.usage.colTokens")}
            </th>
            <th className="text-end font-normal">
              {t("settings.llm.usage.colCost")}
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((b) => (
            <tr key={b.key} className="border-t border-mid-gray/15">
              <td className="py-1 truncate max-w-48">{label(b)}</td>
              <td className="py-1 text-end tabular-nums">
                {formatTokens(b.calls)}
              </td>
              <td className="py-1 text-end tabular-nums">
                {formatTokens(b.prompt_tokens + b.completion_tokens)}
              </td>
              <td className="py-1 text-end tabular-nums">
                {formatUsd(b.cost_micro)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

export default UsageOverview;
