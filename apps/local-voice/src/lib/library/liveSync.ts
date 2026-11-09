import type { SyncPoint } from "@/bindings";

/**
 * Hörbuch-Sync (Stufe 3): Stützpunkte aus dem Live-Abgleich mit denen aus
 * einem Transkript zusammenführen und entscheiden, welches Fenster als
 * Nächstes erkannt werden soll. Reine Funktionen, ohne Tauri.
 */

export interface LiveSpan {
  from_ms: number;
  to_ms: number;
}

/** So weit voraus wird erkannt (ms). */
export const LOOKAHEAD_MS = 90_000;
/** Länge eines Fensters (ms). */
export const WINDOW_MS = 30_000;

const finite = (p: SyncPoint) =>
  Number.isFinite(p.chars) && Number.isFinite(p.ms);

/** Längste in Zeit und Zeichen streng steigende Kette (O(n log n)): ein
 *  Ausreißer kostet nur sich selbst, nicht alle Punkte danach. */
export function monotoneChain(points: SyncPoint[]): SyncPoint[] {
  const pts = points
    .filter(finite)
    .sort((a, b) => a.ms - b.ms || b.chars - a.chars);
  const tails: number[] = [];
  const prev: number[] = new Array(pts.length).fill(-1);
  for (let i = 0; i < pts.length; i++) {
    const c = pts[i].chars;
    let lo = 0;
    let hi = tails.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (pts[tails[mid]].chars < c) lo = mid + 1;
      else hi = mid;
    }
    prev[i] = lo > 0 ? tails[lo - 1] : -1;
    tails[lo] = i;
  }
  const out: SyncPoint[] = [];
  for (let i = tails.length ? tails[tails.length - 1] : -1; i >= 0; i = prev[i])
    out.push(pts[i]);
  return out.reverse();
}

/** Punkte, die in beiden Achsen zwischen die Nachbarn in `fixed` passen. */
function fitting(fixed: SyncPoint[], points: SyncPoint[]): SyncPoint[] {
  if (!fixed.length) return points.filter(finite);
  return points.filter(finite).filter((p) => {
    // Erster fester Punkt mit ms >= p.ms.
    let lo = 0;
    let hi = fixed.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (fixed[mid].ms < p.ms) lo = mid + 1;
      else hi = mid;
    }
    const before = fixed[lo - 1];
    const after = fixed[lo];
    if (after && after.ms === p.ms) return false;
    return (
      (!before || p.chars > before.chars) && (!after || p.chars < after.chars)
    );
  });
}

/**
 * Quellen nach Vorrang zusammenführen, streng steigend in Zeichen und Zeit:
 * jede spätere Schicht zählt nur in den Lücken der früheren, wo sie in beiden
 * Achsen zwischen die Nachbarn passt. Reihenfolge: Hand-Stellen, Transkript,
 * Live-Abgleich.
 */
export function mergeLayers(
  ...layers: (SyncPoint[] | null | undefined)[]
): SyncPoint[] {
  let fixed: SyncPoint[] = [];
  for (const layer of layers) {
    if (!layer?.length) continue;
    fixed = monotoneChain([...fixed, ...fitting(fixed, layer)]);
  }
  return fixed;
}

/** Transkript (Vorrang) und Live-Abgleich zusammenführen. */
export const mergeSync = (
  base: SyncPoint[] | null | undefined,
  live: SyncPoint[],
): SyncPoint[] => mergeLayers(base, live);

/** Hand-Stellen in der Nähe von `ms` (Zeitachse dort verlässlich). */
export const ANCHOR_TRUST_MS = 120_000;
export const nearAnchor = (anchors: SyncPoint[], ms: number) =>
  anchors.some((a) => Math.abs(a.ms - ms) <= ANCHOR_TRUST_MS);

/** Zeitraum, der `ms` enthält, oder null. */
export function spanAt(spans: LiveSpan[], ms: number): LiveSpan | null {
  for (const s of spans) if (ms >= s.from_ms && ms < s.to_ms) return s;
  return null;
}

/**
 * Nächstes zu erkennendes Fenster ab der Abspielstelle, oder null, wenn bis
 * `LOOKAHEAD_MS` voraus alles abgedeckt ist (oder das Ende erreicht ist).
 */
export function nextWindow(
  spans: LiveSpan[],
  positionMs: number,
  totalMs: number,
): { from: number; len: number } | null {
  let from = positionMs;
  // Über zusammenhängende Abdeckung ab der Abspielstelle springen.
  for (let s = spanAt(spans, from); s; s = spanAt(spans, from)) from = s.to_ms;
  if (from >= positionMs + LOOKAHEAD_MS) return null;
  if (totalMs > 0 && from >= totalMs - 500) return null;
  const len = totalMs > 0 ? Math.min(WINDOW_MS, totalMs - from) : WINDOW_MS;
  return len >= 1_000 ? { from, len } : null;
}

/** Abdeckung nach einem Fenster ergänzen (zusammenhängende Bereiche verschmelzen). */
export function addSpan(spans: LiveSpan[], span: LiveSpan): LiveSpan[] {
  if (!(span.to_ms > span.from_ms)) return spans;
  const all = [...spans, span].sort((a, b) => a.from_ms - b.from_ms);
  const out: LiveSpan[] = [];
  for (const s of all) {
    const last = out[out.length - 1];
    if (last && s.from_ms <= last.to_ms + 1)
      last.to_ms = Math.max(last.to_ms, s.to_ms);
    else out.push({ ...s });
  }
  // Nichts dazugekommen: dieselbe Liste, damit kein Effekt neu läuft.
  const same =
    out.length === spans.length &&
    out.every(
      (s, i) => s.from_ms === spans[i].from_ms && s.to_ms === spans[i].to_ms,
    );
  return same ? spans : out;
}
