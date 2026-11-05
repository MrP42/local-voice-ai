import type { AppSettings, LlmConnection, LlmModelConfig } from "@/bindings";

/**
 * Cloud-Freigabe für das Buch-Gespräch (lva#309, Plan §3.8): wohin gesendet
 * würde und was das kostet. Rein; die Oberfläche fragt, bevor die erste
 * Buchstelle den Rechner verlässt.
 */

/** Funktion in „Modelle je Funktion“ (wie `FN_BOOK_TALK` in Rust). */
export const BOOK_TALK_FUNCTION = "book_talk";

/** Annahme je Frage: Eingabe (Anweisung, ~7.000 Zeichen Auszüge, zwei Wechsel, Frage) und Antwort in Token. */
export const ESTIMATE_INPUT_TOKENS = 2800;
export const ESTIMATE_OUTPUT_TOKENS = 350;

export interface BookModel {
  model: LlmModelConfig;
  connection: LlmConnection;
}

type ModelSettings = Pick<
  AppSettings,
  "function_models" | "llm_active_model_id" | "llm_models" | "llm_connections"
>;

/**
 * Das Modell, das das Buch-Gespräch benutzt: die Wahl der Funktion, wenn sie
 * freigegeben und ihre Verbindung an ist, sonst das Standardmodell (wie
 * `for_function` in Rust).
 */
export function bookModelOf(s: Partial<ModelSettings> | null): BookModel | null {
  if (!s) return null;
  const connections = s.llm_connections ?? [];
  const usable = (id: string | null | undefined): BookModel | null => {
    const model = (s.llm_models ?? []).find(
      (m) => m.id === id && m.enabled !== false,
    );
    const connection = connections.find(
      (c) => c.id === model?.connection_id && c.enabled !== false,
    );
    return model && connection ? { model, connection } : null;
  };
  return (
    usable(s.function_models?.[BOOK_TALK_FUNCTION]?.model_id) ??
    usable(s.llm_active_model_id)
  );
}

/** Platzhalter-Adresse des lokalen Servers (`managers::llm::LOCAL_PLACEHOLDER_URL`). */
const LOCAL_PLACEHOLDER_URL = "http://127.0.0.1:0/v1";

/** Läuft das Modell über den lokalen Server? Dann verlässt nichts den Rechner (wie `is_local` in Rust). */
export const isLocalConnection = (c: LlmConnection): boolean =>
  c.kind === "local" ||
  c.base_url.trim().replace(/\/+$/, "") ===
    LOCAL_PLACEHOLDER_URL.replace(/\/+$/, "");

/**
 * Schlüssel der Freigabe: Verbindung und Modell, nicht der Anbietertyp (Plan
 * §3.8, §11.4 „je Modell“). Gleiche Form wie `consent_key` im Kern: zwei
 * Verbindungen desselben Typs oder ein anderes Modell brauchen eine eigene Freigabe.
 */
export const consentKeyOf = (b: BookModel): string =>
  `${b.connection.id}::${b.model.remote_id}`;

/** Das Modell, das eine Freigabe braucht und noch keine hat; `null`, wenn gesendet werden darf. */
export function pendingConsent(
  s: Partial<ModelSettings & Pick<AppSettings, "book_talk_cloud_consent">> | null,
): BookModel | null {
  const found = bookModelOf(s);
  if (!found || isLocalConnection(found.connection)) return null;
  const list = s?.book_talk_cloud_consent ?? [];
  return list.includes(consentKeyOf(found)) ? null : found;
}

/** Geschätzte Kosten einer Frage in USD; `null`, wenn der Preis nicht hinterlegt ist. */
export function estimateAskCostUsd(model: LlmModelConfig): number | null {
  const input = model.price_input_per_mtok;
  const output = model.price_output_per_mtok;
  if (typeof input !== "number" || typeof output !== "number") return null;
  return (
    (ESTIMATE_INPUT_TOKENS * input + ESTIMATE_OUTPUT_TOKENS * output) / 1_000_000
  );
}

/** Freigabeliste mit dem Schlüssel, ohne Doppelte. */
export function withConsent(list: string[] | undefined, key: string): string[] {
  return [...new Set([...(list ?? []), key])];
}

/** Freigabeliste ohne den Schlüssel (Widerruf). */
export function withoutConsent(
  list: string[] | undefined,
  key: string,
): string[] {
  return (list ?? []).filter((k) => k !== key);
}
