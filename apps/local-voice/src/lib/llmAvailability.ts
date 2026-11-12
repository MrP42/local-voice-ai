import { useEffect, useState } from "react";
import { commands, type LlmAvailability } from "@/bindings";

const EMPTY: LlmAvailability = {
  unavailable_connection_ids: [],
  fallback_model_id: null,
};

let cached: LlmAvailability = EMPTY;
const listeners = new Set<(a: LlmAvailability) => void>();
let pending: Promise<void> | null = null;

/** Fragt neu, ob die Abo-Verbindungen auf diesem Rechner laufen (CLI installiert). */
export function refreshLlmAvailability(): Promise<void> {
  if (pending) return pending;
  pending = commands
    .llmAvailability()
    .then((a) => {
      cached = a;
      listeners.forEach((l) => l(a));
    })
    .catch(() => undefined)
    .finally(() => {
      pending = null;
    });
  return pending;
}

/**
 * Welche Verbindungen auf diesem Rechner nicht laufen (Abo ohne installierte
 * CLI) und welches Modell die App stattdessen nutzt -- dieselbe Auflösung wie
 * `AppSettings::active_llm_model` in Rust. `key` (z. B. die Verbindungen als
 * JSON) löst bei Änderungen eine neue Abfrage aus; dazu jedes Fokussieren.
 */
export function useLlmAvailability(key = ""): LlmAvailability {
  const [state, setState] = useState(cached);
  useEffect(() => {
    listeners.add(setState);
    void refreshLlmAvailability();
    const onFocus = () => void refreshLlmAvailability();
    window.addEventListener("focus", onFocus);
    return () => {
      listeners.delete(setState);
      window.removeEventListener("focus", onFocus);
    };
  }, [key]);
  return state;
}
