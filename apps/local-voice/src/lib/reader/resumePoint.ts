/**
 * Weiterlesen ab der letzten Stelle (lva#148): je Seite und Reiter merkt
 * sich der Reader, wo zuletzt vorgelesen wurde. Gespeichert wird der
 * Quell-Offset samt einem kurzen Textanker; ändert sich der Text davor,
 * findet der Anker die Stelle wieder, statt mitten in ein Wort zu springen.
 */
import {
  docProgressKey,
  getLocal,
  recordPosition,
  setLocal,
} from "../library/progress";

/** Unter `lva.ui.` – und dauerhaft im Kern gespiegelt (lva#152). */
const PREFIX = "tts.resume.";
const ANCHOR = 40;

interface Stored {
  offset: number;
  anchor: string;
}

export function saveResumePoint(
  key: string,
  text: string,
  offset: number,
): void {
  if (offset <= 0 || offset >= text.length) return;
  const value: Stored = { offset, anchor: anchorAt(text, offset) };
  setLocal(PREFIX + key, JSON.stringify(value));
  // Fortschritt der Bibliothek: was gehört wurde, bleibt gehört (lva#152).
  recordPosition(progressKeyOf(key), {
    to: offset,
    length: text.length,
    anchor: value.anchor,
  });
}

/**
 * Fortschrittsschlüssel zu einem Weiterlese-Schlüssel: KI-Dokumente
 * (`<seite>:doc:<id>`, lva#183) zählen als abgeleitetes Dokument des Buchs,
 * alles andere als Text der Seite.
 */
export function progressKeyOf(key: string): string {
  const m = /^(.+):doc:([^:]+)$/.exec(key);
  return m ? docProgressKey(m[1], m[2]) : `text:${key}`;
}

export function clearResumePoint(key: string): void {
  // Nur die Stelle zum Weiterlesen; der Fortschritt (gehört, fertig) bleibt.
  setLocal(PREFIX + key, null);
}

/** Offset der gemerkten Stelle im aktuellen Text, oder null. */
export function loadResumePoint(key: string, text: string): number | null {
  const raw = getLocal(PREFIX + key);
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
