import type { MeetingSpeaker, StoredSegment } from "@/bindings";

/**
 * #242: Tabelle "Sprecher & Redeanteile" mit Hörproben und Umbenennen.
 * Reine Hilfsfunktionen ohne React (Auswahl der Hörproben, Abschnitt im
 * Protokoll finden, Zahlen formatieren).
 */

/**
 * Sprechernummer eines ganzen Kanals ohne Sprechertrennung ("Aufnahme",
 * "Ich", "Gegenseite"); wie `speakers::WHOLE_CHANNEL` im Backend.
 */
export const WHOLE_CHANNEL = 0;

/** Überschrift des Abschnitts im Protokoll (Backend `minutes_to_markdown`). */
export const SPEAKER_SECTION_HEADING = "## Sprecher & Redeanteile";

/** So viele Hörproben je Sprecher. */
export const MAX_SAMPLES = 6;
/** So lang spielt eine Hörprobe höchstens. */
export const MAX_SAMPLE_MS = 10_000;
/** Kürzere Sätze nur, wenn es keine längeren gibt. */
const MIN_SAMPLE_MS = 2_000;

export interface SpeakerSample {
  segment_index: number;
  channel: number;
  start_ms: number;
  end_ms: number;
}

export const speakerKey = (s: { channel: number; speaker_index: number }) =>
  `${s.channel}:${s.speaker_index}`;

/** Der Sprecher eines Segments (ohne Sprecher: der ganze Kanal). */
export const segmentSpeakerIndex = (segment: StoredSegment) =>
  segment.speaker_index ?? WHOLE_CHANNEL;

/**
 * Typische Ausschnitte eines Sprechers: über die ganze Besprechung verteilt
 * (die Sätze in zeitlicher Reihenfolge in gleich große Gruppen geteilt) und je
 * Gruppe der längste Satz, höchstens `MAX_SAMPLE_MS` lang. Sätze unter zwei
 * Sekunden nur, wenn der Sprecher keine längeren hat.
 */
export function speakerSamples(
  segments: StoredSegment[],
  speaker: { channel: number; speaker_index: number },
  max = MAX_SAMPLES,
): SpeakerSample[] {
  const own = segments
    .filter(
      (s) =>
        s.channel === speaker.channel &&
        segmentSpeakerIndex(s) === speaker.speaker_index &&
        s.end_ms > s.start_ms,
    )
    .sort((a, b) => a.start_ms - b.start_ms);
  const long = own.filter((s) => s.end_ms - s.start_ms >= MIN_SAMPLE_MS);
  const pool = long.length > 0 ? long : own;
  const count = Math.min(max, pool.length);
  const out: SpeakerSample[] = [];
  for (let group = 0; group < count; group++) {
    const from = Math.floor((group * pool.length) / count);
    const to = Math.floor(((group + 1) * pool.length) / count);
    let best = pool[from];
    for (let i = from + 1; i < to; i++) {
      if (pool[i].end_ms - pool[i].start_ms > best.end_ms - best.start_ms) {
        best = pool[i];
      }
    }
    out.push({
      segment_index: best.segment_index,
      channel: best.channel,
      start_ms: best.start_ms,
      end_ms: Math.min(best.end_ms, best.start_ms + MAX_SAMPLE_MS),
    });
  }
  return out;
}

/** Zeilen der Tabelle: wer gesprochen hat, in der Reihenfolge des ersten Auftretens. */
export function shareRows(speakers: MeetingSpeaker[]): MeetingSpeaker[] {
  return speakers
    .filter((s) => s.speech_ms > 0)
    .sort(
      (a, b) =>
        (a.first_ms ?? Number.MAX_SAFE_INTEGER) -
          (b.first_ms ?? Number.MAX_SAFE_INTEGER) ||
        a.channel - b.channel ||
        a.speaker_index - b.speaker_index,
    );
}

/**
 * Den Abschnitt "Sprecher & Redeanteile" im Protokoll finden: Überschrift und
 * die Tabelle darunter. `null`, wenn es ihn nicht gibt (ein Sprecher, ältere
 * Dokumente ohne Tabelle).
 */
export function splitSpeakerSection(
  markdown: string,
): { before: string; heading: string; after: string } | null {
  const lines = markdown.split("\n");
  const start = lines.findIndex(
    (line) => line.trim() === SPEAKER_SECTION_HEADING,
  );
  if (start < 0) return null;
  let end = start + 1;
  while (end < lines.length && lines[end].trim() === "") end++;
  if (end >= lines.length || !lines[end].trim().startsWith("|")) return null;
  while (end < lines.length && lines[end].trim().startsWith("|")) end++;
  return {
    before: lines.slice(0, start).join("\n"),
    heading: lines[start].trim(),
    after: lines.slice(end).join("\n"),
  };
}

/**
 * Das Protokoll ohne den Abschnitt "Sprecher & Redeanteile": Namen, Zeiten und
 * Prozentwerte werden nicht vorgelesen (auch nicht in der Audiodatei).
 */
export function withoutSpeakerSection(markdown: string): string {
  const parts = splitSpeakerSection(markdown);
  if (!parts) return markdown;
  return [parts.before, parts.after]
    .filter((part) => part.trim() !== "")
    .join("\n\n");
}

/** Redezeit wie im Protokoll: "05:09", ab einer Stunde "1:02:05". */
export function formatSpeechTime(ms: number): string {
  const total = Math.round(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

/** Anteil mit einer Nachkommastelle in der Sprache der Oberfläche ("10,4 %"). */
export function formatShare(pct: number, language: string): string {
  const number = new Intl.NumberFormat(language, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(pct);
  return `${number} %`;
}
