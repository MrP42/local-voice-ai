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

/** Beide Quellen, streng steigend in Zeichen und Zeit. Transkript-Punkte
 *  bleiben, Live-Punkte füllen die Lücken dazwischen. */
export function mergeSync(
  base: SyncPoint[] | null | undefined,
  live: SyncPoint[],
): SyncPoint[] {
  const all = [...(base ?? []), ...live].filter(
    (p) => Number.isFinite(p.chars) && Number.isFinite(p.ms),
  );
  all.sort((a, b) => a.ms - b.ms || a.chars - b.chars);
  const out: SyncPoint[] = [];
  for (const p of all) {
    const last = out[out.length - 1];
    if (!last || (p.chars > last.chars && p.ms > last.ms)) out.push(p);
  }
  return out;
}

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
  const all = [...spans, span].sort((a, b) => a.from_ms - b.from_ms);
  const out: LiveSpan[] = [];
  for (const s of all) {
    const last = out[out.length - 1];
    if (last && s.from_ms <= last.to_ms + 1)
      last.to_ms = Math.max(last.to_ms, s.to_ms);
    else out.push({ ...s });
  }
  return out;
}
