import { useEffect, useState } from "react";
import {
  commands,
  type LlmAvailability,
  type LlmConnection,
  type LlmModelConfig,
} from "@/bindings";

const EMPTY: LlmAvailability = {
  unavailable_connection_ids: [],
  fallback_model_id: null,
};

let cached: LlmAvailability = EMPTY;
const listeners = new Set<(a: LlmAvailability) => void>();
let pending: Promise<void> | null = null;
let again = false;

/**
 * Fragt neu, ob die Abo-Verbindungen auf diesem Rechner laufen (CLI
 * installiert). Kommt ein Anlass, während eine Abfrage läuft, folgt danach
 * eine weitere -- die laufende kennt die neue Lage womöglich nicht.
 */
export function refreshLlmAvailability(): Promise<void> {
  if (pending) {
    again = true;
    return pending;
  }
  pending = commands
    .llmAvailability()
    .then((a) => {
      cached = a;
      listeners.forEach((l) => l(a));
    })
    .catch(() => undefined)
    .finally(() => {
      pending = null;
      if (again) {
        again = false;
        void refreshLlmAvailability();
      }
    });
  return pending;
}

/**
 * Schlüssel für [`useLlmAvailability`] aus allem, was die Auflösung ändert:
 * Verbindungen (Art, Adresse, Schalter, Training), Modelle (Freigabe),
 * Standardwahl und Regelwerk.
 */
export function availabilityKey(
  connections: LlmConnection[],
  models: LlmModelConfig[],
  activeId: string | null,
  profile: unknown,
): string {
  return JSON.stringify([
    connections.map((c) => [
      c.id,
      c.kind,
      c.base_url,
      c.enabled,
      c.training_opt_out,
    ]),
    models.map((m) => [m.id, m.connection_id, m.enabled]),
    activeId,
    profile,
  ]);
}

/**
 * Welche Verbindungen auf diesem Rechner nicht laufen (Abo ohne installierte
 * CLI) und welches Modell die App stattdessen nutzt -- dieselbe Auflösung wie
 * `AppSettings::active_llm_model` in Rust. `key` ([`availabilityKey`]) löst
 * bei Änderungen eine neue Abfrage aus; dazu jedes Fokussieren.
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
