import type { SyncPoint } from "@/bindings";

/**
 * Mitlesen (lva#204): Zeitachse zwischen Hörbuch (ms) und Text (Zeichen).
 * Mit `sync` aus dem Transkript ist sie genau (je Absatz ein Stützpunkt,
 * dazwischen linear); ohne (EPUB-Text zu einem Hörbuch) wird anteilig
 * abgebildet und die Stelle gilt als „ungefähr“.
 */

export interface FollowMap {
  /** Anteilig geschätzt statt aus dem Transkript. */
  approx: boolean;
  msToChars(ms: number): number;
  charsToMs(chars: number): number;
}

interface Pair {
  chars: number;
  ms: number;
}

/** Zwischen den Stützpunkten linear; `from`/`to` wählen die Achsen. */
function interpolate(
  points: Pair[],
  value: number,
  from: "chars" | "ms",
  to: "chars" | "ms",
): number {
  if (value < points[0][from]) return points[0][to];
  const last = points[points.length - 1];
  if (value >= last[from]) return last[to];
  let lo = 0;
  let hi = points.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (points[mid][from] <= value) lo = mid;
    else hi = mid;
  }
  const a = points[lo];
  const b = points[hi];
  const span = b[from] - a[from];
  if (span <= 0) return a[to];
  return a[to] + ((value - a[from]) / span) * (b[to] - a[to]);
}

export function makeFollowMap(
  sync: SyncPoint[] | null | undefined,
  textLength: number,
  totalMs: number,
): FollowMap {
  const clean = (sync ?? [])
    .filter((p) => Number.isFinite(p.chars) && Number.isFinite(p.ms))
    .sort((a, b) => a.chars - b.chars);
  // Monoton in beiden Achsen, sonst läuft die Umkehrung ins Leere.
  const points: Pair[] = [{ chars: 0, ms: 0 }];
  for (const p of clean) {
    const prev = points[points.length - 1];
    if (p.chars < prev.chars) continue;
    points.push({ chars: p.chars, ms: Math.max(p.ms, prev.ms) });
  }
  const endMs = Math.max(totalMs, points[points.length - 1].ms);
  points.push({
    chars: Math.max(textLength, points[points.length - 1].chars),
    ms: endMs,
  });
  return {
    approx: clean.length === 0,
    msToChars: (ms) => Math.round(interpolate(points, ms, "ms", "chars")),
    charsToMs: (chars) => interpolate(points, chars, "chars", "ms"),
  };
}

export interface TextParagraph {
  /** Beginn im Text (Zeichen). */
  start: number;
  end: number;
  text: string;
  heading: boolean;
}

/** Absätze (durch Leerzeilen getrennt) samt Beginn im Text. */
export function splitParagraphs(text: string): TextParagraph[] {
  const out: TextParagraph[] = [];
  const re = /\n{2,}/g;
  let start = 0;
  const push = (end: number) => {
    const raw = text.slice(start, end);
    if (raw.trim()) {
      const heading = /^#{1,6}\s/.test(raw);
      out.push({
        start,
        end,
        text: heading ? raw.replace(/^#{1,6}\s+/, "") : raw,
        heading,
      });
    }
  };
  for (let m = re.exec(text); m; m = re.exec(text)) {
    push(m.index);
    start = m.index + m[0].length;
  }
  push(text.length);
  return out;
}

/** Index des Absatzes, in dem `chars` liegt (der letzte davor, falls dazwischen). */
export function paragraphAt(
  paragraphs: TextParagraph[],
  chars: number,
): number {
  let lo = 0;
  let hi = paragraphs.length - 1;
  let found = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (paragraphs[mid].start <= chars) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/**
 * Startstelle, wenn der Text neuer ist als das Hören: der Textfortschritt
 * zählt nur, wenn er klar jünger ist (das Hören sichert beide in einem Zug).
 */
export const TEXT_NEWER_MS = 2000;
export const textIsNewer = (textUpdated: number, audioUpdated: number) =>
  textUpdated > audioUpdated + TEXT_NEWER_MS;
