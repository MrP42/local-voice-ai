import { invoke } from "@tauri-apps/api/core";
import { useSettingsStore } from "@/stores/settingsStore";

/**
 * Lernen aus Korrekturen und Einrichtung der Erkennung (Stufe 3, ADR-0021).
 * Die Befehle liegen im Kern unter `src-tauri/src/speech_learning/`; Typen und Werte
 * folgen ihm. Texte und Paare gehören nie ins Log.
 */

/** Bereich der Paare; vorerst gibt es nur einen. Muster `[a-z0-9_-]{1,32}`. */
export const LEARNING_BEREICH = "standard";
/** Längster Text für `feedback` und `setup_evaluate` (Zeichen). */
export const MAX_LEARNING_TEXT = 2000;

export type LearnedStatus =
  "beobachtet" | "vorschlag" | "uebernommen" | "abgelehnt";

export interface LearnedPair {
  id: number;
  bereich: string;
  gehoert: string;
  korrigiert: string;
  anzahl: number;
  zuletzt: string;
  status: LearnedStatus;
}

export interface FeedbackOutcome {
  reason: "ok" | "nothing" | "rewrite" | "off";
  learned: number;
  suggestions: number;
  adopted: number;
}

export interface SetupText {
  sentences: string[];
  text: string;
}

export interface SetupOutcome {
  reason: "ok" | "nothing" | "rewrite";
  matched_percent: number;
  pairs: LearnedPair[];
}

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
    return await invoke<FeedbackOutcome>("speech_learning_feedback", {
      bereich: LEARNING_BEREICH,
      erkannt: heard,
      korrigiert: corrected,
    });
  } catch {
    return null;
  }
}

export const listPairs = () =>
  invoke<LearnedPair[]>("speech_learning_list", { bereich: LEARNING_BEREICH });

export const setPairStatus = (id: number, status: LearnedStatus) =>
  invoke<LearnedPair>("speech_learning_set_status", { id, status });

export const deletePair = (id: number) =>
  invoke<null>("speech_learning_delete", { id });

export const clearPairs = () =>
  invoke<number>("speech_learning_clear", { bereich: LEARNING_BEREICH });

export const getSetupText = () =>
  invoke<SetupText>("speech_learning_setup_text");

export const prepareSetup = () => invoke<null>("speech_learning_setup_prepare");

export const evaluateSetup = (reference: string, transcript: string) =>
  invoke<SetupOutcome>("speech_learning_setup_evaluate", {
    bereich: LEARNING_BEREICH,
    reference,
    transcript,
  });

/** Der Wortschatz hat sich im Kern geändert: Einstellungen neu laden (Liste „Eigene Wörter“). */
export const refreshWords = () => useSettingsStore.getState().refreshSettings();
