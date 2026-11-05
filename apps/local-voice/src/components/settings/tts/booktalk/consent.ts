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

/**
 * Anbieterkennung der Freigabe: so führt der Kern `book_talk_cloud_consent`
 * (Anbieter-Kennung = `kind` der Verbindung, wie `post_process_provider_id`).
 */
export const providerIdOf = (c: LlmConnection): string => c.kind;

/** Geschätzte Kosten einer Frage in USD; `null`, wenn der Preis nicht hinterlegt ist. */
export function estimateAskCostUsd(model: LlmModelConfig): number | null {
  const input = model.price_input_per_mtok;
  const output = model.price_output_per_mtok;
  if (typeof input !== "number" || typeof output !== "number") return null;
  return (
    (ESTIMATE_INPUT_TOKENS * input + ESTIMATE_OUTPUT_TOKENS * output) / 1_000_000
  );
}

/** Freigabeliste mit dem Anbieter, ohne Doppelte. */
export function withConsent(list: string[] | undefined, id: string): string[] {
  return [...new Set([...(list ?? []), id])];
}
