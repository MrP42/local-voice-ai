import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type AuditEntry, type IntegrationView } from "@/bindings";
import { Select, type SelectOption } from "../ui/Select";
import { capabilityKey } from "./model";

export interface AuditFilter {
  integration: string;
  outcome: string;
  caller: string;
}

export const EMPTY_AUDIT_FILTER: AuditFilter = {
  integration: "",
  outcome: "",
  caller: "",
};

interface AuditViewProps {
  views: IntegrationView[];
  filter: AuditFilter;
  onFilterChange: (filter: AuditFilter) => void;
}

const OUTCOMES = ["ok", "denied", "error", "pending"] as const;
const CALLERS = ["user", "workflow", "agent_external", "agent_local"] as const;

const outcomeClass: Record<string, string> = {
  ok: "bg-green-500/15 text-status-green",
  denied: "bg-amber-500/15 text-status-amber",
  error: "bg-red-500/15 text-status-red",
  pending: "bg-mid-gray/20 text-text",
};

interface Detail {
  phase?: string;
  reason?: string;
  count?: number;
  error?: string;
  mode?: string;
  for?: string;
  decision?: string;
}

const parseDetail = (json: string | null): Detail => {
  if (!json) return {};
  try {
    const parsed = JSON.parse(json) as unknown;
    return parsed && typeof parsed === "object" ? (parsed as Detail) : {};
  } catch {
    return {};
  }
};

/**
 * Protokoll: jede Aktion eines Workflows oder Agenten ueber das Register, dazu
 * die Aenderungen des Nutzers (Rechte, Richtung, Freigaben). Neueste zuerst.
 * Das Backend begrenzt die Aufbewahrung (20 000 Zeilen, Verweigerungen
 * zuerst verdraengt) und schwaerzt Geheimnisse; hier steht nur Klartext.
 */
export const AuditView: React.FC<AuditViewProps> = ({
  views,
  filter,
  onFilterChange,
}) => {
  const { t, i18n } = useTranslation();
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setFailed(false);
    void commands
      .integrationsAuditList(
        filter.integration || null,
        filter.outcome || null,
        filter.caller || null,
        200,
      )
      .then((result) => {
        if (cancelled) return;
        if (result.status === "ok") setEntries(result.data ?? []);
        else setFailed(true);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [filter.integration, filter.outcome, filter.caller]);

  const label = (id: string | null) => {
    if (!id) return t("integrations.audit.noIntegration");
    return views.find((v) => v.integration.id === id)?.integration.label ?? id;
  };

  const describe = (entry: AuditEntry): string => {
    const d = parseDetail(entry.detail_json);
    const parts: string[] = [];
    if (d.phase === "grant_changed") {
      parts.push(
        t("integrations.audit.phase.grant_changed", {
          mode:
            d.mode === "default"
              ? t("integrations.audit.default")
              : t(`integrations.modes.${d.mode ?? "off"}`, {
                  defaultValue: d.mode ?? "",
                }),
          caller: t(`integrations.callers.${d.for ?? ""}`, {
            defaultValue: d.for ?? "",
          }),
        }),
      );
    } else if (d.phase === "approval_decided") {
      parts.push(
        t("integrations.audit.phase.approval_decided", {
          decision: t(`integrations.audit.decision.${d.decision ?? ""}`, {
            defaultValue: d.decision ?? "",
          }),
        }),
      );
    } else if (d.phase) {
      parts.push(
        t(`integrations.audit.phase.${d.phase}`, { defaultValue: d.phase }),
      );
    }
    if (d.reason) {
      parts.push(
        t(`integrations.audit.reason.${d.reason}`, { defaultValue: d.reason }),
      );
    }
    if (d.error) parts.push(d.error);
    if (d.count && d.count > 1) parts.push(`${d.count}×`);
    return parts.join(" · ");
  };

  const filters: {
    key: keyof AuditFilter;
    label: string;
    options: SelectOption[];
  }[] = [
    {
      key: "integration",
      label: t("integrations.audit.filterIntegration"),
      options: views.map((v) => ({
        value: v.integration.id,
        label: v.integration.label,
      })),
    },
    {
      key: "outcome",
      label: t("integrations.audit.filterOutcome"),
      options: OUTCOMES.map((o) => ({
        value: o,
        label: t(`integrations.audit.outcome.${o}`),
      })),
    },
    {
      key: "caller",
      label: t("integrations.audit.filterCaller"),
      options: CALLERS.map((c) => ({
        value: c,
        label: t(`integrations.callers.${c}`),
      })),
    },
  ];

  return (
    <div className="space-y-3" data-testid="audit">
      <div className="flex flex-wrap items-end gap-3">
        {filters.map(({ key, label: text, options }) => (
          <div
            key={key}
            className="flex min-w-[11rem] flex-col gap-1"
            data-testid={`audit-filter-${key}`}
          >
            <span className="text-xs text-text-muted">{text}</span>
            <Select
              value={filter[key] || null}
              options={options}
              placeholder={t("integrations.audit.all")}
              ariaLabel={text}
              menuPortal
              onChange={(value) =>
                onFilterChange({ ...filter, [key]: value ?? "" })
              }
            />
          </div>
        ))}
      </div>

      {failed && (
        <p className="text-sm text-status-red" role="alert">
          {t("integrations.audit.loadError")}
        </p>
      )}
      {entries && entries.length === 0 && !failed && (
        <p className="text-sm text-text-muted" data-testid="audit-empty">
          {t("integrations.audit.empty")}
        </p>
      )}
      {entries && entries.length > 0 && (
        <ul className="divide-y divide-mid-gray/20 rounded-lg border border-mid-gray/20">
          {entries.map((entry) => {
            const detail = describe(entry);
            return (
              <li
                key={entry.id}
                className="grid gap-x-3 gap-y-1 px-3 py-2 text-sm sm:grid-cols-[9.5rem_1fr_auto]"
                data-testid="audit-row"
              >
                <time
                  className="text-xs text-text-muted"
                  dateTime={new Date(entry.ts).toISOString()}
                >
                  {new Date(entry.ts).toLocaleString(i18n.language, {
                    dateStyle: "short",
                    timeStyle: "medium",
                  })}
                </time>
                <div className="min-w-0">
                  <div className="break-words">
                    <span className="font-medium" data-testid="audit-caller">
                      {t(`integrations.callers.${entry.caller}`, {
                        defaultValue: entry.caller,
                      })}
                    </span>
                    <span className="text-text-muted"> → </span>
                    <span>{label(entry.integration_id)}</span>
                    {entry.capability && (
                      <span className="text-text-muted">
                        {" · "}
                        {t(`${capabilityKey(entry.capability)}.title`, {
                          defaultValue: entry.capability,
                        })}
                      </span>
                    )}
                  </div>
                  {entry.target && (
                    <div className="text-xs text-text-muted break-all">
                      {entry.target}
                    </div>
                  )}
                  {detail && (
                    <div className="text-xs text-text-muted break-words">
                      {detail}
                    </div>
                  )}
                </div>
                <span
                  className={`h-fit w-fit rounded-full px-2 py-0.5 text-xs font-medium ${
                    outcomeClass[entry.outcome] ?? outcomeClass.pending
                  }`}
                  data-testid="audit-outcome"
                >
                  {t(`integrations.audit.outcome.${entry.outcome}`, {
                    defaultValue: entry.outcome,
                  })}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
};
