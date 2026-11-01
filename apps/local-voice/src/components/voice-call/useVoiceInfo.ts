import { useEffect, useMemo, useState } from "react";
import { commands, type LlmConnection, type LlmModelConfig } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { useModelStore } from "@/stores/modelStore";
import type { CallUsage } from "@/stores/voiceCallStore";
import { SUBSCRIPTION_PROVIDERS } from "../settings/meetings/DocUsageLine";
import { speakerModelId } from "./orchestration";

export interface ActiveLlm {
  label: string | null;
  kind: string | null;
  local: boolean;
}

/** Der Sprecher des Gesprächs: Modell der Funktion „Sprachgespräch“, sonst das Standardmodell. */
export function useActiveLlm(): ActiveLlm {
  const { getSetting } = useSettings();
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = speakerModelId({
    function_models: getSetting("function_models"),
    llm_active_model_id: getSetting("llm_active_model_id"),
    llm_models: models,
    llm_connections: connections,
  });
  return useMemo(() => {
    const model = models.find((m) => m.id === activeId);
    const connection = model
      ? connections.find((c) => c.id === model.connection_id)
      : undefined;
    if (!model || !connection) return { label: null, kind: null, local: false };
    return {
      label: modelLabel(model.remote_id, model.label),
      kind: connection.kind,
      local: connection.kind === "local",
    };
  }, [connections, models, activeId]);
}

/** Name des aktuellen Spracherkennungsmodells. */
export function useSttModelName(): string | null {
  const { models, currentModel } = useModelStore();
  return models.find((m) => m.id === currentModel)?.name ?? null;
}

export type CallCost =
  | { kind: "subscription" }
  | { kind: "local" }
  | { kind: "usd"; usd: number }
  | { kind: "unknown" }
  | { kind: "none" };

/**
 * Kosten des Gesprächs. Abo und lokal kosten nichts; nur API-Aufrufe werden
 * über das Ledger abgefragt. Ein API-Aufruf ohne Preis (Kosten 0 bei Token > 0)
 * gilt als „nicht erfasst“, nicht als kostenlos.
 */
export function useCallCost(usage: CallUsage, llm: ActiveLlm): CallCost {
  const { subscription, local, api } = usage.calls;
  const key = usage.billableEventIds.join(",");
  const [micro, setMicro] = useState<number | null>(null);

  useEffect(() => {
    setMicro(null);
    if (api === 0 || !key) return;
    let cancelled = false;
    const ids = key.split(",").map(Number);
    void commands
      .usageCostOf(ids)
      .then((r) => {
        if (!cancelled && r.status === "ok") setMicro(r.data);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [key, api]);

  if (subscription + local + api === 0) {
    // Noch keine Antwort: das aktive Modell sagt, was zu erwarten ist.
    if (llm.kind !== null && SUBSCRIPTION_PROVIDERS.has(llm.kind))
      return { kind: "subscription" };
    return llm.local ? { kind: "local" } : { kind: "none" };
  }
  if (api === 0) {
    if (local === 0) return { kind: "subscription" };
    if (subscription === 0) return { kind: "local" };
    return { kind: "usd", usd: 0 };
  }
  if (
    micro === null ||
    (micro === 0 && usage.promptTokens + usage.completionTokens > 0)
  )
    return { kind: "unknown" };
  return { kind: "usd", usd: micro / 1_000_000 };
}

export const formatUsd = (usd: number, lang: string): string =>
  `${usd.toLocaleString(lang, { minimumFractionDigits: 2, maximumFractionDigits: 4 })} $`;

export function median(values: number[]): number {
  if (values.length === 0) return 0;
  const s = [...values].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}
