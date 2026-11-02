/**
 * Hörstelle beim Zusammenführen (lva#204): die Stellen der beteiligten Bücher
 * (je in Millisekunden ihres eigenen Hörbuchs) werden auf die Gesamtzeit des
 * zusammengeführten Buchs umgerechnet. Es gilt die jüngste Stelle.
 */

export interface MergeEntry {
  /** Dauer je Teil in ms; `null` = unbekannt. */
  durations: (number | null)[];
  /** Gesicherte Hörstelle des Buchs (ms), falls vorhanden. */
  progress: { offset: number; updated: number; finished?: boolean } | null;
}

export function mergedPosition(
  entries: MergeEntry[],
): { ms: number; updated: number } | null {
  let base: number | null = 0;
  let best: { ms: number; updated: number } | null = null;
  for (const entry of entries) {
    const p = entry.progress;
    if (base !== null && p && !p.finished && p.offset > 0) {
      if (!best || p.updated > best.updated)
        best = { ms: base + p.offset, updated: p.updated };
    }
    // Ohne bekannte Dauern lässt sich der Beginn der folgenden Bücher nicht bestimmen.
    if (base !== null) {
      const known = entry.durations.every((d) => d !== null && d > 0);
      base = known
        ? base + entry.durations.reduce<number>((a, d) => a + (d ?? 0), 0)
        : null;
    }
  }
  return best;
}
