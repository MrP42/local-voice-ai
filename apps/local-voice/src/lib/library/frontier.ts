/**
 * Lesegrenze des Buch-Gesprächs (lva#309, Plan §3.0): `frontier` = weitester
 * gelesener oder gehörter Stand eines Textes (UTF-16-Offset im Originaltext).
 * Zurückblättern und Zurückspulen senken sie nie. Rein und ohne Abhängigkeiten,
 * damit `node --test` sie ohne die App prüfen kann.
 */

/** Der Teil eines Fortschritts-Datensatzes, den die Grenze braucht. */
export interface FrontierSource {
  frontier?: number;
  heard?: [number, number][];
  read?: [number, number][];
}

const finite = (n: unknown): n is number =>
  typeof n === "number" && Number.isFinite(n) && n >= 0;

/**
 * Grenze eines Datensatzes: das Größte aus gespeicherter Grenze, Ende der
 * gehörten und Ende der gelesenen Bereiche. Die aktuelle Stelle (`offset`)
 * zählt nicht: ein Kapitelsprung ist weder gehört noch gelesen.
 */
export function frontierOf(rec: FrontierSource | null | undefined): number {
  if (!rec) return 0;
  let best = finite(rec.frontier) ? rec.frontier : 0;
  for (const [, end] of [...(rec.heard ?? []), ...(rec.read ?? [])])
    if (finite(end) && end > best) best = end;
  return best;
}

/** Grenze des neuen Datensatzes, nie niedriger als die des bisherigen und die eigenen Bereiche. */
export function steadyFrontier(
  next: FrontierSource,
  prev: FrontierSource | null | undefined,
): number {
  return Math.max(frontierOf(next), frontierOf(prev));
}

/** Grenze für die Anfrage: `undefined` solange nichts gelesen oder gehört wurde (Kern: `no_frontier`). */
export function requestFrontier(
  rec: FrontierSource | null | undefined,
): number | undefined {
  const f = frontierOf(rec);
  return f > 0 ? f : undefined;
}
