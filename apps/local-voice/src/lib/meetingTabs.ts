/**
 * Reiter der Aufnahmen-Seite (G4, #70).
 *
 * - Mitte: Transkript, Protokoll (bei zwei Fassungen: Vergleich; sobald es Folien
 *   aus einem Video gibt: Folien).
 * - Rechts unter der Bedienung: Notizen, KI-Notizen, Fragen.
 *
 * Vorher (bis 0.20.9) standen Notizen, KI-Notizen und Protokoll in der Mitte
 * (`meetings.midTab`) und Transkript oder Fragen rechts (`meetings.rightTab`).
 * Die neuen Reiter haben eigene Schluessel; die alten Werte werden beim ersten
 * Start sinngemaess uebernommen.
 */

export type CenterTab = "transcript" | "minutes" | "compare" | "slides";
export type NotesTab = "notes" | "ai";
export type LowerTab = NotesTab | "chat";

export const CENTER_TAB_KEY = "meetings.centerTab";
export const LOWER_TAB_KEY = "meetings.lowerTab";

export const isCenterTab = (value: string): value is CenterTab =>
  value === "transcript" ||
  value === "minutes" ||
  value === "compare" ||
  value === "slides";
export const isLowerTab = (value: string): value is LowerTab =>
  value === "notes" || value === "ai" || value === "chat";

/** Der alte Mitte-Reiter: Protokoll und Vergleich bleiben, Notizen gehen nach rechts. */
export const centerFromLegacy = (midTab: string | null): CenterTab =>
  midTab === "minutes" || midTab === "compare" ? midTab : "transcript";

/** Der alte Reiter rechts ("chat") gewinnt, sonst der alte Notizen-Reiter der Mitte. */
export const lowerFromLegacy = (
  midTab: string | null,
  rightTab: string | null,
): LowerTab =>
  rightTab === "chat" ? "chat" : midTab === "ai" ? "ai" : "notes";

/** Gespeicherter Wert eines alten Schluessels (ohne Fehler, wenn der Speicher fehlt). */
export const readLegacyTab = (key: string): string | null => {
  try {
    return window.localStorage.getItem(`lva.ui.${key}`);
  } catch {
    return null;
  }
};
