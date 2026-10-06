import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type ProvenanceEntry } from "@/bindings";
import { displayModelName, splitModel } from "@/lib/modelNames";

/** Abo ueber die CLI: keine API-Kosten (abgerechnet im Abo). */
const SUBSCRIPTION_PROVIDERS = new Set(["claude_cli", "codex_cli"]);

const num = (n: number) => n.toLocaleString("de-DE");

/**
 * Kopfzeile eines erzeugten Dokuments (Protokoll): Modell, Effort, Token und
 * Kosten. Abo-Modelle kosten nichts extra — das Sternchen erklärt es beim
 * Überfahren; lokale Modelle ebenso.
 */
export const DocUsageLine: React.FC<{ documentId: string }> = ({
  documentId,
}) => {
  const { t } = useTranslation();
  const [entry, setEntry] = useState<ProvenanceEntry | null>(null);
  const [cost, setCost] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    setEntry(null);
    setCost(null);
    (async () => {
      try {
        const result = await commands.provenanceGet("document", documentId);
        if (cancelled || result.status !== "ok") return;
        const e = (result.data ?? [])[0] ?? null;
        setEntry(e);
        if (!e) return;
        let ids: number[] = [];
        try {
          const params = JSON.parse(e.params_json ?? "{}") as {
            usage_event_ids?: unknown;
          };
          if (Array.isArray(params.usage_event_ids))
            ids = params.usage_event_ids.filter(
              (x): x is number => typeof x === "number",
            );
        } catch {
          /* ohne Parameter keine Kosten */
        }
        if (ids.length === 0 && e.usage_event_id != null)
          ids = [e.usage_event_id];
        if (ids.length > 0) {
          const micro = await commands.usageCostOf(ids);
          if (!cancelled && micro.status === "ok") setCost(micro.data);
        }
      } catch {
        // Ohne Backend (Browser-Test) keine Zeile.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [documentId]);

  if (!entry || (!entry.model_id && !entry.model_label)) return null;
  const [model, effort] = splitModel(entry.model_id ?? entry.model_label ?? "");
  const label = displayModelName(model || entry.model_label || "");
  const pin = entry.prompt_tokens ?? 0;
  const pout = entry.completion_tokens ?? 0;
  const subscription = SUBSCRIPTION_PROVIDERS.has(entry.provider ?? "");
  const local = entry.locality === "local";

  let costText: string;
  let costTitle: string | undefined;
  if (subscription) {
    costText = "0,00 $*";
    costTitle = t("meetings.docUsage.subscriptionHint");
  } else if (local) {
    costText = "0,00 $*";
    costTitle = t("meetings.docUsage.localHint");
  } else if (cost != null) {
    costText = `${(cost / 1_000_000).toLocaleString("de-DE", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 4,
    })} $`;
  } else {
    costText = "–";
    costTitle = t("meetings.docUsage.costUnknown");
  }

  return (
    <p
      className="text-xs text-text/60 flex flex-wrap gap-x-3 gap-y-0.5"
      data-testid="doc-usage"
    >
      <span>
        {t("meetings.docUsage.model")}:{" "}
        <span className="text-text/80">{label}</span>
      </span>
      {effort && (
        <span data-testid="doc-usage-effort">
          {t("meetings.docUsage.effort")}:{" "}
          <span className="text-text/80">
            {t(`settings.llm.effortLevels.${effort}`, { defaultValue: effort })}
          </span>
        </span>
      )}
      {pin + pout > 0 && (
        <span
          title={t("meetings.docUsage.tokensHint", {
            input: num(pin),
            output: num(pout),
          })}
        >
          {t("meetings.docUsage.tokens")}:{" "}
          <span className="text-text/80 tabular-nums">{num(pin + pout)}</span>
        </span>
      )}
      <span title={costTitle} data-testid="doc-usage-cost">
        {t("meetings.docUsage.cost")}:{" "}
        <span
          className={`text-text/80 tabular-nums ${costTitle ? "underline decoration-dotted cursor-help" : ""}`}
        >
          {costText}
        </span>
      </span>
    </p>
  );
};
