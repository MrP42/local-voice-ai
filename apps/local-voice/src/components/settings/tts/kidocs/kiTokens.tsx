import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Coins } from "lucide-react";
import {
  commands,
  type BookAiEstimate,
  type KiUsage,
  type SummarySource,
  type TokenFigures,
} from "@/bindings";

/** Token als ganze Zahl mit Tausenderpunkten, große Werte gerundet. */
export function formatTokens(n: number, locale: string): string {
  const rounded =
    n >= 100_000
      ? Math.round(n / 1000) * 1000
      : n >= 10_000
        ? Math.round(n / 100) * 100
        : n;
  return rounded.toLocaleString(locale);
}

/** Betrag in US-Dollar ohne Einheit; unter einem Cent „< 0,01“. */
export function formatCost(micro: number, locale: string): string {
  const usd = micro / 1_000_000;
  if (usd > 0 && usd < 0.01) return `< ${(0.01).toLocaleString(locale)}`;
  return usd.toLocaleString(locale, {
    minimumFractionDigits: 2,
    maximumFractionDigits: usd < 1 ? 3 : 2,
  });
}

/** Modellname aus „anbieter/modell“. */
export const modelName = (key: string): string =>
  key.includes("/") ? key.slice(key.indexOf("/") + 1) : key;

function costText(
  f: TokenFigures,
  local: boolean,
  model: string,
  t: TFunction,
  locale: string,
): string {
  if (local) return t("tts.kiDocs.tokens.local");
  if (f.cost_micro === null) return t("tts.kiDocs.tokens.noPrice");
  return t("tts.kiDocs.tokens.cost", {
    cost: formatCost(f.cost_micro, locale),
    model: model || "?",
  });
}

/** Ausführliche Zeile für Tooltip und Screenreader. */
export function figuresDetail(
  f: TokenFigures,
  t: TFunction,
  locale: string,
): string {
  return t("tts.kiDocs.tokens.detail", {
    input: formatTokens(f.input_tokens, locale),
    output: formatTokens(f.output_tokens, locale),
    count: f.calls,
  });
}

/**
 * Prognose im Dialog „Neues KI-Dokument“: fragt den Kern nach einer
 * Trockenfahrt des Ablaufs (gleiche Aufteilung wie der echte Lauf, Cache
 * berücksichtigt). Erst nach kurzer Ruhe, damit das Durchklicken der
 * Vorlagen nicht für jede eine Rechnung startet.
 */
export function useTokenEstimate(
  pageId: string | null,
  source: SummarySource | null,
  templateId: string | null,
  enabled: boolean,
): { estimate: BookAiEstimate | null; loading: boolean } {
  const [estimate, setEstimate] = useState<BookAiEstimate | null>(null);
  const [loading, setLoading] = useState(false);
  const key = JSON.stringify([pageId, source, templateId, enabled]);
  useEffect(() => {
    if (!enabled || !pageId || !source || !templateId) {
      setEstimate(null);
      setLoading(false);
      return;
    }
    let alive = true;
    // Keine Zahlen der vorigen Auswahl stehen lassen, während neu gerechnet wird.
    setEstimate(null);
    setLoading(true);
    const timer = window.setTimeout(() => {
      commands
        .bookAiEstimate(pageId, source, templateId)
        .then((r) => {
          if (!alive) return;
          setEstimate(r.status === "ok" ? r.data : null);
        })
        .catch(() => alive && setEstimate(null))
        .finally(() => alive && setLoading(false));
    }, 250);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
    // `key` fasst alle Eingaben zusammen (die Quelle ist ein neues Objekt je Render).
  }, [key]);
  return { estimate, loading };
}

/** Dezente Prognosezeile unter der Laufzeitschätzung im Dialog. */
export function TokenEstimateLine({
  estimate,
  loading,
}: {
  estimate: BookAiEstimate | null;
  loading: boolean;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  if (!estimate) {
    return loading ? (
      <p className="kidoc-tokens" data-testid="kidoc-token-estimate">
        <Coins className="kidoc-tokens__icon" aria-hidden="true" />
        {t("tts.kiDocs.tokens.estimating")}
      </p>
    ) : null;
  }
  const f = estimate.figures;
  return (
    <p
      className="kidoc-tokens"
      data-testid="kidoc-token-estimate"
      title={`${figuresDetail(f, t, locale)}\n${t(
        estimate.measured
          ? "tts.kiDocs.tokens.measured"
          : "tts.kiDocs.tokens.average",
      )}`}
    >
      <Coins className="kidoc-tokens__icon" aria-hidden="true" />
      <span>
        {t("tts.kiDocs.tokens.estimate", {
          total: formatTokens(f.input_tokens + f.output_tokens, locale),
        })}
        <span className="kidoc-tokens__sub">
          {" · "}
          {t("tts.kiDocs.tokens.split", {
            input: formatTokens(f.input_tokens, locale),
            output: formatTokens(f.output_tokens, locale),
          })}
          {" · "}
          {costText(f, estimate.local, estimate.model, t, locale)}
          {estimate.cached_steps > 0 &&
            ` · ${t("tts.kiDocs.tokens.cached", { count: estimate.cached_steps })}`}
        </span>
      </span>
    </p>
  );
}

/** Kurzform am Dokument: „51 200 Token“; leer ohne Messung. */
export function usageShort(
  usage: KiUsage | null | undefined,
  t: TFunction,
  locale: string,
): string | null {
  const a = usage?.actual;
  if (!a || a.calls === 0) return null;
  const total = a.input_tokens + a.output_tokens;
  if (total === 0) return t("tts.kiDocs.tokens.notReported");
  return t("tts.kiDocs.tokens.short", { total: formatTokens(total, locale) });
}

/**
 * Ist neben Prognose in der Kopfzeile eines offenen KI-Dokuments: was der Lauf
 * gebraucht hat und was vorher geschätzt war, mit Abweichung in Prozent.
 */
export function TokenUsageLine({
  usage,
  model,
}: {
  usage: KiUsage | null | undefined;
  /** „anbieter/modell“ des Dokuments. */
  model: string;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const a = usage?.actual;
  if (!usage || !a || a.calls === 0) return null;
  const total = a.input_tokens + a.output_tokens;
  const e = usage.estimate;
  const planned = e ? e.input_tokens + e.output_tokens : 0;
  const delta =
    planned > 0 && total > 0
      ? Math.round(((total - planned) / planned) * 100)
      : null;
  const lines = [figuresDetail(a, t, locale)];
  if (e) {
    lines.push(
      t("tts.kiDocs.tokens.plannedDetail", {
        detail: figuresDetail(e, t, locale),
      }),
    );
  }
  return (
    <p
      className="kidoc-tokens kidoc-tokens--usage"
      data-testid="kidoc-token-usage"
      title={lines.join("\n")}
    >
      <Coins className="kidoc-tokens__icon" aria-hidden="true" />
      {total === 0
        ? t("tts.kiDocs.tokens.notReported")
        : t("tts.kiDocs.tokens.actual", {
            total: formatTokens(total, locale),
          })}
      {e && planned > 0 && (
        <span className="kidoc-tokens__sub">
          {" · "}
          {t(
            delta === null
              ? "tts.kiDocs.tokens.vsEstimate"
              : "tts.kiDocs.tokens.vsEstimateDelta",
            {
              planned: formatTokens(planned, locale),
              delta: `${delta !== null && delta > 0 ? "+" : delta !== null && delta < 0 ? "−" : "±"}${Math.abs(delta ?? 0)}`,
            },
          )}
        </span>
      )}
      <span className="kidoc-tokens__sub">
        {" · "}
        {costText(a, usage.local, modelName(model), t, locale)}
      </span>
    </p>
  );
}
