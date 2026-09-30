// Reine Logik der Protokoll-Erzeugung (P1k, B14): keine React-, keine Tauri-
// Importe, damit sie sich ohne Browser pruefen laesst (`node`, Type-Stripping).
//
// Die Typen kommen aus `bindings.ts` (nur als Typ importiert, wird beim
// Ausfuehren entfernt).

import type { MinutesPhase, MinutesProgress } from "@/bindings";

/** Codes, die das Backend fuer Fehler der Protokoll-Erzeugung liefert (`minutes.rs`). */
export const MINUTES_ERROR_CODES = [
  "minutes_busy",
  "minutes_cancelled",
  "meeting_not_found",
  "meeting_not_finished",
  "no_transcript",
  "template_not_found",
  "no_provider",
  "no_model",
  "memory_low",
  "llm_failed",
  "store_failed",
] as const;

export type MinutesErrorCode = (typeof MINUTES_ERROR_CODES)[number];

/**
 * Der Code einer Fehlermeldung (`"<code>"` oder `"<code>: <detail>"`).
 * Unbekanntes gilt als `llm_failed`, wie im Backend.
 */
export const minutesErrorCode = (error: string): MinutesErrorCode => {
  const found = MINUTES_ERROR_CODES.find(
    (code) => error === code || error.startsWith(`${code}:`),
  );
  return found ?? "llm_failed";
};

/** Der Text hinter dem Code (`"<code>: <detail>"`), sonst die ganze Meldung. */
export const minutesErrorDetail = (error: string): string => {
  const code = MINUTES_ERROR_CODES.find((c) => error.startsWith(`${c}:`));
  return code ? error.slice(code.length + 1).trim() : error;
};

/**
 * Fortschritt in ganzen Prozent; `null`, wenn er unbestimmt ist (`total` 0:
 * waehrend der Vorlagenwahl). Nie ueber 100, nie unter 0.
 */
export const progressPercent = (
  progress: Pick<MinutesProgress, "done" | "total"> | null | undefined,
): number | null => {
  if (!progress || progress.total <= 0) return null;
  const percent = Math.round((progress.done / progress.total) * 100);
  return Math.min(100, Math.max(0, percent));
};

/** Schluessel des Phasentexts. */
export const phaseKey = (phase: MinutesPhase): string =>
  `meetings.minutes.phase.${phase}`;
