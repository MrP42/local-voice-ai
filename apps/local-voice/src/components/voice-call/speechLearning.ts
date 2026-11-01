import { commands } from "@/bindings";
import type {
  FeedbackOutcome,
  LearnedPair,
  SetupOutcome,
  SetupText,
} from "@/bindings";
import { useSettingsStore } from "@/stores/settingsStore";

export type { FeedbackOutcome, LearnedPair, SetupOutcome, SetupText };

/**
 * Lernen aus Korrekturen und Einrichtung der Erkennung (Stufe 3, ADR-0021).
 * Die Befehle liegen im Kern unter `src-tauri/src/speech_learning/`. Texte und Paare
 * gehören nie ins Log.
 */

/** Bereich der Paare; vorerst gibt es nur einen. Muster `[a-z0-9_-]{1,32}`. */
export const LEARNING_BEREICH = "standard";
/** Längster Text für `feedback` und `setup_evaluate` (Zeichen). */
export const MAX_LEARNING_TEXT = 2000;

/** Status eines Paares (`LearnedPair.status` ist im Kern ein Text). */
export type LearnedStatus =
  "beobachtet" | "vorschlag" | "uebernommen" | "abgelehnt";

const STATUSES: LearnedStatus[] = [
  "beobachtet",
  "vorschlag",
  "uebernommen",
  "abgelehnt",
];

export function statusOf(pair: LearnedPair): LearnedStatus {
  return STATUSES.find((s) => s === pair.status) ?? "beobachtet";
}

/** Packt das Ergebnis eines Befehls aus; ein Fehler wird geworfen (nur der Text des Kerns). */
function unwrap<T>(
  r: { status: "ok"; data: T } | { status: "error"; error: string },
): T {
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

/** Fehlertext für die Anzeige. */
export const errorText = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

/** Ist „Aus Korrekturen lernen“ an? (Standard aus.) */
export function learningEnabled(): boolean {
  return useSettingsStore.getState().settings?.speech_learning_enabled === true;
}

/**
 * Meldet eine Korrektur an den Lernspeicher. Nur bei eingeschaltetem Lernen und nur,
 * wenn beide Texte in die Grenze passen. Fehler bleiben still: Die Korrektur selbst ist
 * schon gespeichert, das Lernen ist ein Zusatz.
 */
export async function sendFeedback(
  heard: string,
  corrected: string,
): Promise<FeedbackOutcome | null> {
  if (!learningEnabled()) return null;
  if (heard.length > MAX_LEARNING_TEXT || corrected.length > MAX_LEARNING_TEXT)
    return null;
  try {
    return unwrap(
      await commands.speechLearningFeedback(LEARNING_BEREICH, heard, corrected),
    );
  } catch {
    return null;
  }
}

export const listPairs = async () =>
  unwrap(await commands.speechLearningList(LEARNING_BEREICH));

export const setPairStatus = async (id: number, status: LearnedStatus) =>
  unwrap(await commands.speechLearningSetStatus(id, status));

export const deletePair = async (id: number) =>
  unwrap(await commands.speechLearningDelete(id));

export const clearPairs = async () =>
  unwrap(await commands.speechLearningClear(LEARNING_BEREICH));

export const getSetupText = () => commands.speechLearningSetupText();

export const prepareSetup = async () =>
  unwrap(await commands.speechLearningSetupPrepare());

export const evaluateSetup = async (reference: string, transcript: string) =>
  unwrap(
    await commands.speechLearningSetupEvaluate(
      LEARNING_BEREICH,
      reference,
      transcript,
    ),
  );

/** Der Wortschatz hat sich im Kern geändert: Einstellungen neu laden (Liste „Eigene Wörter“). */
export const refreshWords = () => useSettingsStore.getState().refreshSettings();
