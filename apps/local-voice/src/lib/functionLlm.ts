import { useMemo } from "react";
import type { FunctionModel, LlmConnection, LlmModelConfig } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { availabilityKey, useLlmAvailability } from "@/lib/llmAvailability";
import { modelLabel } from "@/lib/modelNames";

export interface FunctionLlm {
  /** Anzeigename des Modells, `null` ohne wählbares Modell. */
  model: string | null;
  /** Name der Verbindung (Anbieter). */
  connection: string | null;
  /** Läuft auf diesem Rechner (Verbindungsart `local`). */
  local: boolean;
}

/**
 * Welches Sprachmodell eine Funktion (`function_models`, z. B. `read_aloud`)
 * nutzt: der eigene Eintrag, sonst das aktive Modell -- dieselbe Auflösung wie
 * `FunctionModelChip` und `for_function` in Rust. Für Hinweise vor dem Start,
 * wo der Text verarbeitet wird. Eine Abo-Verbindung ohne installierte CLI
 * zählt nicht; dann gilt der Ersatz (lokales Modell zuerst), wie in Rust.
 */
export function useFunctionLlm(fn: string): FunctionLlm {
  const { getSetting } = useSettings();
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
  const entries = (getSetting("function_models") ?? {}) as Partial<
    Record<string, FunctionModel>
  >;
  const ownId = entries[fn]?.model_id ?? null;
  const { unavailable_connection_ids: missing, fallback_model_id: fallbackId } =
    useLlmAvailability(
      availabilityKey(
        connections,
        models,
        activeId,
        getSetting("compliance_profile"),
      ),
    );
  return useMemo(() => {
    const on = new Map(
      connections
        .filter((c) => c.enabled !== false && !missing.includes(c.id))
        .map((c) => [c.id, c]),
    );
    const pick = (id: string | null) => {
      const model = models.find(
        (m) => m.id === id && m.enabled !== false && on.has(m.connection_id),
      );
      return model ? { model, connection: on.get(model.connection_id)! } : null;
    };
    // Wie Rust: der Ersatz gilt nur für eine gültige Standardwahl, die hier
    // nicht läuft; ohne Ersatz bleibt diese Wahl (der Aufruf meldet dann).
    const chosen = models.find(
      (m) =>
        m.id === activeId &&
        m.enabled !== false &&
        connections.some(
          (c) => c.id === m.connection_id && c.enabled !== false,
        ),
    );
    const chosenShown = chosen
      ? (() => {
          const connection = connections.find(
            (c) => c.id === chosen.connection_id,
          )!;
          return { model: chosen, connection };
        })()
      : null;
    const shown =
      pick(ownId) ??
      pick(activeId) ??
      (chosenShown ? (pick(fallbackId) ?? chosenShown) : null);
    if (!shown) return { model: null, connection: null, local: false };
    return {
      model: modelLabel(shown.model.remote_id, shown.model.label),
      connection: shown.connection.label,
      local: shown.connection.kind === "local",
    };
  }, [connections, models, activeId, ownId, missing, fallbackId]);
}
