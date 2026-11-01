/**
 * Weiterlesen ab der letzten Stelle (lva#148): je Seite und Reiter merkt
 * sich der Reader, wo zuletzt vorgelesen wurde. Gespeichert wird der
 * Quell-Offset samt einem kurzen Textanker; ändert sich der Text davor,
 * findet der Anker die Stelle wieder, statt mitten in ein Wort zu springen.
 */
const PREFIX = "lva.ui.tts.resume.";
const ANCHOR = 40;

interface Stored {
  offset: number;
  anchor: string;
}

const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

export function saveResumePoint(
  key: string,
  text: string,
  offset: number,
): void {
  if (offset <= 0 || offset >= text.length) return;
  const value: Stored = { offset, anchor: anchorAt(text, offset) };
  try {
    storage()?.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    /* voller Speicher: Weiterlesen ist eine Bequemlichkeit */
  }
}

export function clearResumePoint(key: string): void {
  try {
    storage()?.removeItem(PREFIX + key);
  } catch {
    /* ignorieren */
  }
}

/** Offset der gemerkten Stelle im aktuellen Text, oder null. */
export function loadResumePoint(key: string, text: string): number | null {
  let raw: string | null = null;
  try {
    raw = storage()?.getItem(PREFIX + key) ?? null;
  } catch {
    return null;
  }
  if (!raw) return null;
  let stored: Stored;
  try {
    stored = JSON.parse(raw) as Stored;
  } catch {
    return null;
  }
  if (typeof stored?.offset !== "number" || typeof stored.anchor !== "string") {
    return null;
  }
  const at = resolveAnchor(text, stored.offset, stored.anchor);
  return at !== null && at > 0 ? at : null;
}

/** Textanker ab `offset` (für Lesezeichen und Weiterlesen). */
export const anchorAt = (text: string, offset: number): string =>
  text.slice(offset, offset + ANCHOR);

/**
 * Stelle des Ankers im aktuellen Text: an der alten Position, sonst das
 * nächstgelegene Vorkommen (Text davor geändert), sonst null.
 */
export function resolveAnchor(
  text: string,
  offset: number,
  anchor: string,
): number | null {
  if (!anchor) return null;
  if (text.startsWith(anchor, offset)) return offset;
  let best: number | null = null;
  for (let i = text.indexOf(anchor); i >= 0; i = text.indexOf(anchor, i + 1)) {
    if (best === null || Math.abs(i - offset) < Math.abs(best - offset))
      best = i;
  }
  return best;
}
