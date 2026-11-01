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

export function saveResumePoint(key: string, text: string, offset: number): void {
  if (offset <= 0 || offset >= text.length) return;
  const value: Stored = { offset, anchor: text.slice(offset, offset + ANCHOR) };
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
  if (!stored.anchor) return null;
  if (text.startsWith(stored.anchor, stored.offset)) return stored.offset;
  // Text davor geändert: den Anker suchen, das nächstgelegene Vorkommen gewinnt.
  let best: number | null = null;
  for (let i = text.indexOf(stored.anchor); i >= 0; i = text.indexOf(stored.anchor, i + 1)) {
    if (best === null || Math.abs(i - stored.offset) < Math.abs(best - stored.offset)) {
      best = i;
    }
  }
  return best !== null && best > 0 ? best : null;
}
