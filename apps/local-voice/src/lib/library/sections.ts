import type { AudioTrack, TranscriptPart } from "@/bindings";
import type { TranscriptSegment } from "./transcript.ts";

/**
 * Lange Einzeldatei in Zeitabschnitten transkribieren (lva#208). Reine
 * Funktionen ohne Tauri-Abhängigkeit (laufen auch in den Node-Tests).
 *
 * Ein Hörbuch aus nur einer Datei, die länger als {@link SECTION_THRESHOLD_MS}
 * ist, wird in N = min(4, ⌈Dauer / 20 min⌉) Abschnitte geteilt. Jeder
 * Abschnitt ist ein eigener Auftrag der Warteschlange, der nur seinen Bereich
 * der Datei liest. Damit an den Grenzen kein Wort zerschnitten wird, reicht
 * jeder Bereich {@link SECTION_OVERLAP_MS} über seine Grenze hinaus; beim
 * Zusammenführen gehört jedes Segment dem Abschnitt, in dessen Grenzen seine
 * Mitte liegt, und ein doppelt erkannter Satz an der Naht fällt weg.
 */

/** Erst eine Datei über dieser Dauer wird geteilt. */
export const SECTION_THRESHOLD_MS = 40 * 60_000;
/** Zielgröße eines Abschnitts. */
export const SECTION_TARGET_MS = 20 * 60_000;
/** So viele Abschnitte laufen höchstens gleichzeitig (Grenze der Warteschlange). */
export const MAX_SECTIONS = 4;
/** Überlappung an jeder inneren Grenze. */
export const SECTION_OVERLAP_MS = 2_000;
/** Reserve hinter der erwarteten Dauer: ffmpeg endet ohnehin am Dateiende. */
const END_MARGIN_MS = 60_000;

/** Ein zu startender Abschnitt: Bereich der Datei und Zuständigkeit. */
export interface SectionPlan {
  /** Beginn des dekodierten Bereichs (ms). */
  start_ms: number;
  /** Ende des dekodierten Bereichs (ms). */
  end_ms: number;
  /** Zuständig ab hier (ms). */
  from_ms: number;
  /** Zuständig bis hier (ausschließlich); `null` beim letzten Abschnitt. */
  to_ms: number | null;
}

/**
 * Abschnitte für eine Datei der Dauer `durationMs`. Leer: nicht teilen (Dauer
 * unbekannt oder höchstens 40 min); der Teil läuft dann als ein Auftrag.
 */
export function planSections(
  durationMs: number | null | undefined,
): SectionPlan[] {
  if (
    !durationMs ||
    !Number.isFinite(durationMs) ||
    durationMs <= SECTION_THRESHOLD_MS
  )
    return [];
  const n = Math.min(MAX_SECTIONS, Math.ceil(durationMs / SECTION_TARGET_MS));
  const bounds = Array.from({ length: n + 1 }, (_, k) =>
    Math.round((durationMs * k) / n),
  );
  return Array.from({ length: n }, (_, k) => {
    const last = k === n - 1;
    return {
      start_ms: k === 0 ? 0 : bounds[k] - SECTION_OVERLAP_MS,
      end_ms: last
        ? Math.ceil(durationMs) + END_MARGIN_MS
        : bounds[k + 1] + SECTION_OVERLAP_MS,
      from_ms: bounds[k],
      to_ms: last ? null : bounds[k + 1],
    };
  });
}

/** Ein Auftrag mit seinem Bereich; ein ganzer Teil hat `start_ms = from_ms = 0`. */
export type TrackJob = TranscriptPart;

/**
 * Alle Aufträge eines Teils: die Abschnitte, sonst der eine Auftrag des
 * ganzen Teils (Altbestand), sonst keine.
 */
export function jobsOfTrack(
  track: Pick<AudioTrack, "transcript_job" | "transcript_jobs">,
): TrackJob[] {
  if (track.transcript_jobs?.length) return track.transcript_jobs;
  return track.transcript_job
    ? [
        {
          job: track.transcript_job,
          start_ms: 0,
          end_ms: null,
          from_ms: 0,
          to_ms: null,
        },
      ]
    : [];
}

/** Die Aufträge eines Teils als ein Vergleichswert (ohne Reihenfolge). */
export const jobKey = (
  track: Pick<AudioTrack, "transcript_job" | "transcript_jobs">,
): string =>
  jobsOfTrack(track)
    .map((p) => p.job)
    .sort()
    .join(",");

/** Hat der Teil in Abschnitte geteilte Aufträge? */
export const isSectioned = (
  track: Pick<AudioTrack, "transcript_jobs">,
): boolean => !!track.transcript_jobs?.length;

const wordsOf = (text: string): string[] =>
  text
    .toLocaleLowerCase("de")
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);

/**
 * Sagen beide Segmente dasselbe? Das kürzere muss (fast) vollständig im
 * längeren stecken: mindestens 80 % seiner Wörter kommen im längeren vor.
 */
function sameSpeech(a: string, b: string): boolean {
  const [short, long] = [wordsOf(a), wordsOf(b)].sort(
    (x, y) => x.length - y.length,
  );
  if (!short.length) return true;
  const pool = new Map<string, number>();
  for (const w of long) pool.set(w, (pool.get(w) ?? 0) + 1);
  let hit = 0;
  for (const w of short) {
    const left = pool.get(w) ?? 0;
    if (left > 0) {
      hit++;
      pool.set(w, left - 1);
    }
  }
  return hit / short.length >= 0.8;
}

/** Ein Abschnitt mit seinen Segmenten (Zeiten relativ zu `start_ms`). */
export interface SectionResult {
  part: TrackJob;
  segments: TranscriptSegment[];
}

/**
 * Segmente der Abschnitte eines Teils zu einer Folge zusammenführen, Zeiten
 * relativ zum Beginn der Datei.
 *
 * 1. Jeder Abschnitt verschiebt seine Segmente um `start_ms`.
 * 2. Ein Segment bleibt nur im Abschnitt, in dessen Zuständigkeit
 *    (`from_ms` ≤ Mitte < `to_ms`) seine Mitte liegt. Damit entsteht an einer
 *    Grenze kein Segment doppelt, solange beide Abschnitte es gleich schneiden.
 * 3. Schneiden sie es verschieden, überlappen zwei Segmente derselben Rede
 *    zeitlich: das längere bleibt, das andere fällt weg.
 */
export function mergeSections(sections: SectionResult[]): TranscriptSegment[] {
  const kept: { segment: TranscriptSegment; section: number }[] = [];
  sections.forEach(({ part, segments }, section) => {
    for (const s of segments) {
      const start = s.start_ms + part.start_ms;
      const end = s.end_ms + part.start_ms;
      const mid = (start + end) / 2;
      if (mid < part.from_ms) continue;
      if (part.to_ms != null && mid >= part.to_ms) continue;
      kept.push({ segment: { ...s, start_ms: start, end_ms: end }, section });
    }
  });
  kept.sort((a, b) => a.segment.start_ms - b.segment.start_ms);
  const out: { segment: TranscriptSegment; section: number }[] = [];
  for (const item of kept) {
    const prev = out[out.length - 1];
    const seam =
      prev &&
      prev.section !== item.section &&
      item.segment.start_ms < prev.segment.end_ms &&
      sameSpeech(prev.segment.text, item.segment.text);
    if (!seam) {
      out.push(item);
      continue;
    }
    if (wordsOf(item.segment.text).length > wordsOf(prev.segment.text).length)
      out[out.length - 1] = item;
  }
  return out.map((o) => o.segment);
}

/** Status eines Auftrags, der erneut gestartet werden muss. */
export const RESTART_STATUSES: ReadonlySet<string> = new Set([
  "failed",
  "cancelled",
  "missing",
]);
