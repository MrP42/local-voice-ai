import { useEffect, useMemo, useState } from "react";
import { commands, type LlmConnection, type LlmModelConfig } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { useModelStore } from "@/stores/modelStore";
import type { CallUsage } from "@/stores/voiceCallStore";
import { SUBSCRIPTION_PROVIDERS } from "../settings/meetings/DocUsageLine";

export interface ActiveLlm {
  label: string | null;
  kind: string | null;
  local: boolean;
}

/** Das aktive Sprachmodell aus den Einstellungen (wie die Fußleiste). */
export function useActiveLlm(): ActiveLlm {
  const { getSetting } = useSettings();
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
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

/** Kosten des Gesprächs: Abo und lokal kosten nichts, sonst fragt das Ledger. */
export function useCallCost(usage: CallUsage, llm: ActiveLlm): CallCost {
  const provider = usage.providerId ?? llm.kind;
  const subscription =
    provider !== null && SUBSCRIPTION_PROVIDERS.has(provider);
  const local = usage.providerId ? usage.local : llm.local;
  const key = usage.usageEventIds.join(",");
  const [micro, setMicro] = useState<number | null>(null);

  useEffect(() => {
    setMicro(null);
    if (subscription || local || !key) return;
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
  }, [key, subscription, local]);

  if (subscription) return { kind: "subscription" };
  if (local) return { kind: "local" };
  if (usage.promptTokens + usage.completionTokens === 0)
    return { kind: "none" };
  if (micro === null) return { kind: "unknown" };
  return { kind: "usd", usd: micro / 1_000_000 };
}

export const formatUsd = (usd: number, lang: string): string =>
  `${usd.toLocaleString(lang, { minimumFractionDigits: 2, maximumFractionDigits: 4 })} $`;

export function median(values: number[]): number {
  if (values.length === 0) return 0;
  const s = [...values].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}
