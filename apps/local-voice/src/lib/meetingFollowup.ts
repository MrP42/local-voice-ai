// M6-P6c: reine Hilfen der Follow-up-Mail (ohne React, damit Tests sie ohne
// Oberfläche prüfen können).

/** Adressen aus dem An-Feld: Komma, Semikolon und Leerraum trennen. */
export const parseAddresses = (value: string): string[] =>
  value
    .split(/[;,\s]+/)
    .map((a) => a.trim())
    .filter(Boolean);

/** Dateiname aus dem Betreff: ohne die Zeichen, die Windows verbietet. */
export const emlFileName = (subject: string): string => {
  const base = subject
    .replace(/[\/:*?"<>|\r\n]+/g, "-")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 80);
  return `${base || "Follow-up"}.eml`;
};

/** Fehlercodes von `meeting_followup_*`, die einen eigenen Text haben. */
export const FOLLOWUP_ERROR_CODES = [
  "followup_empty",
  "followup_no_content",
  "mailto_failed",
  "clipboard_failed",
  "write_failed",
  "path_missing",
] as const;

/**
 * Fehler, bei denen ein neuer Versuch nichts ändert (keine Grundlage für die
 * Mail): der Dialog bietet dann kein „Erneut versuchen“ an.
 */
export const FOLLOWUP_FINAL_ERRORS: readonly string[] = ["followup_no_content"];

export const isFinalFollowupError = (code: string): boolean =>
  FOLLOWUP_FINAL_ERRORS.includes(code);
