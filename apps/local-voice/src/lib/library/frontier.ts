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

/** Die sichtbare Ansicht des Lesers: Anfang und Ende (UTF-16) und wie man dorthin kam. */
export interface ReaderView {
  start: number;
  end: number;
  /** Durch Weiterblättern (Taste, Pfeil, Wischen) um genau eine Ansicht, nicht durch einen Sprung. */
  sequential: boolean;
}

/**
 * Neue Lesegrenze, wenn der Leser eine Ansicht zeigt (Plan §3.0): nur
 * sequentielles Weiterblättern hebt sie, und nur an die Grenze anschließend
 * (Anfang der Ansicht höchstens an der alten Grenze, also die neue Grenze
 * höchstens alte Grenze + eine Ansicht). Sprünge (Inhaltsverzeichnis,
 * Lesezeichen, Suche, Seitenzahl, Belegsprung) heben nie an; die erste
 * Ansicht ab Textanfang zählt immer. Senkt die Grenze nie.
 */
export function liftedFrontier(current: number, view: ReaderView): number {
  const base = finite(current) ? current : 0;
  if (!finite(view.start) || !finite(view.end) || view.end <= base) return base;
  const joins = view.start <= base;
  if (joins && (view.sequential || view.start === 0)) return view.end;
  return base;
}
