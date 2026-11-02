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
/** Überlappung an jeder inneren Grenze (jeder Bereich reicht so weit über sie hinaus). */
export const SECTION_OVERLAP_MS = 12_000;

/** Ein zu startender Abschnitt: Bereich der Datei und Zuständigkeit. */
export interface SectionPlan {
  /** Beginn des dekodierten Bereichs (ms). */
  start_ms: number;
  /** Ende des dekodierten Bereichs (ms); `null` beim letzten Abschnitt: bis zum Dateiende, ohne Obergrenze. */
  end_ms: number | null;
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
      end_ms: last ? null : bounds[k + 1] + SECTION_OVERLAP_MS,
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

/** Ein Wort mit geschätzter Zeit (Segmentzeit anteilig auf die Wörter verteilt). */
interface Word {
  text: string;
  norm: string;
  /** Geschätzte Zeit in der Datei (ms). */
  t: number;
  seg: number;
}

/** Ein Segment eines Abschnitts mit Zeiten in der Datei und seinen Wörtern. */
interface Seg {
  text: string;
  start_ms: number;
  end_ms: number;
  /** Erstes Wort und Zahl der Wörter in der Wortliste des Abschnitts. */
  first: number;
  count: number;
}

interface Prepared {
  words: Word[];
  segs: Seg[];
  part: TrackJob;
}

const normWord = (w: string): string =>
  w.toLocaleLowerCase("de").replace(/[^\p{L}\p{N}]/gu, "");

function prepare({ part, segments }: SectionResult): Prepared {
  const words: Word[] = [];
  const segs: Seg[] = [];
  const sorted = [...segments].sort((a, b) => a.start_ms - b.start_ms);
  for (const s of sorted) {
    const start = s.start_ms + part.start_ms;
    const end = s.end_ms + part.start_ms;
    const tokens = s.text.split(/\s+/).filter(Boolean);
    const first = words.length;
    tokens.forEach((text, i) =>
      words.push({
        text,
        norm: normWord(text),
        t: start + ((i + 0.5) / tokens.length) * (end - start),
        seg: segs.length,
      }),
    );
    segs.push({
      text: s.text,
      start_ms: start,
      end_ms: end,
      first,
      count: tokens.length,
    });
  }
  return { words, segs, part };
}

/** Wörter in einem Abstand, bis zu dem zwei Erkennungen als dieselbe Stelle gelten. */
const MAX_SKEW_MS = 8_000;
/** So viele gleiche Wörter hintereinander braucht eine Übereinstimmung an der Naht. */
const MIN_RUN = 4;

/**
 * Wo die Abschnitte `a` und `b` an der Naht `seam` (ms) ineinander übergehen:
 * `a` behält die Wörter vor `cutA`, `b` die ab `cutB`.
 *
 * Gesucht wird die längste Folge gleicher Wörter, die in beiden Abschnitten im
 * Überlappungsfenster vorkommt (der längste Suffix von `a` vor der Naht, der
 * Präfix von `b` ist, mit etwas Toleranz an den Rändern). Geschnitten wird in
 * der Mitte dieser Folge: jedes Wort steht genau einmal, nichts fehlt.
 * Ohne Treffer: Schnitt an der Segmentgrenze von `b`, die der Naht am nächsten
 * liegt; `a` wird dort anteilig nach Wortzeiten gekürzt.
 */
function findSeam(
  a: Prepared,
  b: Prepared,
  seam: number,
): { cutA: number; cutB: number } {
  const lo = seam - SECTION_OVERLAP_MS - 1_000;
  const hi = seam + SECTION_OVERLAP_MS + 1_000;
  const a0 = Math.max(
    0,
    a.words.findIndex((w) => w.t >= lo),
  );
  const aStart = a.words.some((w) => w.t >= lo) ? a0 : a.words.length;
  let bEnd = b.words.findIndex((w) => w.t > hi);
  if (bEnd < 0) bEnd = b.words.length;
  // Längste gemeinsame Folge (dynamische Programmierung, Fenster ~100 Wörter).
  let best = { len: 0, i: -1, j: -1, dist: Infinity };
  let prev = new Array<number>(bEnd + 1).fill(0);
  for (let i = aStart; i < a.words.length; i++) {
    const cur = new Array<number>(bEnd + 1).fill(0);
    for (let j = 1; j <= bEnd; j++) {
      const wa = a.words[i];
      const wb = b.words[j - 1];
      if (
        !wa.norm ||
        wa.norm !== wb.norm ||
        Math.abs(wa.t - wb.t) > MAX_SKEW_MS
      )
        continue;
      cur[j] = prev[j - 1] + 1;
      const len = cur[j];
      const mid = (wa.t + wb.t) / 2;
      const dist = Math.abs(mid - seam);
      if (len > best.len || (len === best.len && dist < best.dist))
        best = { len, i, j: j - 1, dist };
    }
    prev = cur;
  }
  if (best.len >= MIN_RUN) {
    const half = Math.floor(best.len / 2);
    return {
      cutA: best.i - best.len + 1 + half,
      cutB: best.j - best.len + 1 + half,
    };
  }
  // Rückfall: Segmentgrenze von b nahe der Naht.
  let pick = -1;
  for (let k = 0; k < b.segs.length; k++) {
    if (!b.segs[k].count) continue;
    if (
      pick < 0 ||
      Math.abs(b.segs[k].start_ms - seam) <
        Math.abs(b.segs[pick].start_ms - seam)
    )
      pick = k;
  }
  if (pick < 0) return { cutA: a.words.length, cutB: b.words.length };
  const at = b.segs[pick].start_ms;
  const cutA = a.words.findIndex((w) => w.t >= at);
  return { cutA: cutA < 0 ? a.words.length : cutA, cutB: b.segs[pick].first };
}

/** Segmente des Abschnitts, beschränkt auf die Wörter `[from, to)`; Zeiten anteilig. */
function slice(p: Prepared, from: number, to: number): TranscriptSegment[] {
  const out: TranscriptSegment[] = [];
  for (const s of p.segs) {
    const lo = Math.max(from, s.first);
    const hi = Math.min(to, s.first + s.count);
    if (hi <= lo) continue;
    if (lo === s.first && hi === s.first + s.count) {
      out.push({ text: s.text, start_ms: s.start_ms, end_ms: s.end_ms });
      continue;
    }
    const len = s.end_ms - s.start_ms;
    out.push({
      text: p.words
        .slice(lo, hi)
        .map((w) => w.text)
        .join(" "),
      start_ms: s.start_ms + ((lo - s.first) / s.count) * len,
      end_ms: s.start_ms + ((hi - s.first) / s.count) * len,
    });
  }
  return out;
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
 * Jeder Abschnitt verschiebt seine Segmente um `start_ms`. An jeder Naht
 * (Beginn der Zuständigkeit des nächsten Abschnitts) überlappen die beiden
 * Erkennungen um etwa {@link SECTION_OVERLAP_MS} beidseits; sie werden auf
 * Wortebene zusammengesetzt ({@link findSeam}), sodass kein Satzteil doppelt
 * vorkommt oder fehlt, auch wenn beide Erkennungen verschieden schneiden.
 */
export function mergeSections(sections: SectionResult[]): TranscriptSegment[] {
  const prepared = [...sections]
    .sort((x, y) => x.part.from_ms - y.part.from_ms)
    .map(prepare);
  const keepFrom = prepared.map(() => 0);
  const keepTo = prepared.map((p) => p.words.length);
  for (let k = 0; k + 1 < prepared.length; k++) {
    const seam = prepared[k + 1].part.from_ms;
    const { cutA, cutB } = findSeam(prepared[k], prepared[k + 1], seam);
    keepTo[k] = Math.max(cutA, keepFrom[k]);
    keepFrom[k + 1] = cutB;
  }
  return prepared.flatMap((p, k) => slice(p, keepFrom[k], keepTo[k]));
}

/** Status eines Auftrags, der erneut gestartet werden muss. */
export const RESTART_STATUSES: ReadonlySet<string> = new Set([
  "failed",
  "cancelled",
  "missing",
]);

/** Status, mit dem ein Auftrag fertig ist. */
const READY_STATUSES: ReadonlySet<string> = new Set(["ready", "done"]);

/**
 * Ein Status für alle Aufträge eines Buchs. „Fehlgeschlagen“ (oder „fehlt“)
 * gilt erst, wenn nichts mehr in Arbeit ist: solange andere Abschnitte laufen
 * oder warten, bleibt es „in Arbeit“, damit die Übernahme nicht zu früh endet
 * und das Fortsetzen erst anbietet, wenn alles Übrige durch ist.
 */
export function aggregateStatus(statuses: string[]): string {
  const bad = (s: string) => RESTART_STATUSES.has(s);
  const active = statuses.filter((s) => !bad(s) && !READY_STATUSES.has(s));
  if (active.length) {
    const working = active.some((s) => s !== "queued") || statuses.some((s) => READY_STATUSES.has(s));
    return working ? "processing" : "queued";
  }
  if (statuses.some((s) => s === "failed" || s === "cancelled")) return "failed";
  if (statuses.some((s) => s === "missing")) return "missing";
  if (statuses.length && statuses.every((s) => READY_STATUSES.has(s))) return "ready";
  return statuses[0] ?? "queued";
}
