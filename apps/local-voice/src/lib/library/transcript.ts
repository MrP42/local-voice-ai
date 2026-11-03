import type { SyncPoint } from "@/bindings";

/** Transkript → Lesetext und Zeitachse (ohne Tauri-Abhängigkeit). */

export interface TranscriptSegment {
  text: string;
  start_ms: number;
  end_ms: number;
}

export interface TranscriptParagraph {
  text: string;
  start_ms: number;
  end_ms: number;
}

/**
 * Segmente zu Absätzen zusammenfassen, neue Absätze bei einer Pause über
 * 1,5 s oder nach etwa 700 Zeichen. Jeder Absatz kennt Beginn und Ende im
 * Audio.
 */
export function transcriptParagraphs(
  segments: TranscriptSegment[],
): TranscriptParagraph[] {
  const paragraphs: TranscriptParagraph[] = [];
  let current: TranscriptParagraph | null = null;
  let lastEnd = -1;
  for (const s of [...segments].sort((a, b) => a.start_ms - b.start_ms)) {
    const text = s.text.replace(/\s+/g, " ").trim();
    if (!text) continue;
    const pause = lastEnd >= 0 && s.start_ms - lastEnd > 1500;
    if (current && (pause || current.text.length > 700)) {
      paragraphs.push(current);
      current = null;
    }
    if (current) {
      current.text = `${current.text} ${text}`;
      current.end_ms = s.end_ms;
    } else {
      current = { text, start_ms: s.start_ms, end_ms: s.end_ms };
    }
    lastEnd = s.end_ms;
  }
  if (current) paragraphs.push(current);
  return paragraphs;
}

/** Transkript zu Lesetext: die Absätze durch Leerzeilen getrennt. */
export function transcriptToText(segments: TranscriptSegment[]): string {
  return transcriptParagraphs(segments)
    .map((p) => p.text)
    .join("\n\n");
}

/**
 * Text samt Zeitachse aus Absätzen: `prefix` (Titelei) steht vor dem ersten
 * Absatz; jeder Absatz bekommt einen Stützpunkt (Zeichenstelle im fertigen
 * Text → Millisekunden im gesamten Hörbuch).
 */
export function textWithSync(
  paragraphs: TranscriptParagraph[],
  prefix = "",
): { text: string; sync: SyncPoint[] } {
  let text = prefix;
  const sync: SyncPoint[] = [];
  paragraphs.forEach((p, i) => {
    if (i > 0) text += "\n\n";
    sync.push({ chars: text.length, ms: Math.max(0, Math.round(p.start_ms)) });
    text += p.text;
  });
  return { text, sync };
}

/** Ein Teil mit eigenem Transkript, der in einen gemeinsamen Text eingeht. */
export interface TranscriptPiece {
  /** Überschrift des Teils im gemeinsamen Text (Kapitelliste im Reader). */
  heading: string;
  /** Text des Teils samt seiner Titelei. */
  text: string;
  /** Zeitachse des Teils (Zeichen im eigenen Text → ms im eigenen Hörbuch). */
  sync: SyncPoint[];
  /** Beginn des Teils im zusammengeführten Hörbuch (ms). */
  offsetMs: number;
}

/**
 * Transkripte mehrerer Teile zu **einem** Lesetext verbinden (Zusammenführen):
 * jede Titelei fällt weg (alles vor dem ersten Stützpunkt), jeder Teil bekommt
 * eine Überschrift, und die Zeitachse wird auf Zeichen im neuen Text und auf
 * die Gesamtzeit umgerechnet. Teile ohne Stützpunkte (leer) fallen weg.
 */
export function joinTranscripts(
  prefix: string,
  pieces: TranscriptPiece[],
): { text: string; sync: SyncPoint[] } {
  let text = prefix;
  const sync: SyncPoint[] = [];
  for (const piece of pieces) {
    if (!piece.sync.length) continue;
    const points = [...piece.sync].sort((a, b) => a.chars - b.chars);
    const start = Math.min(points[0].chars, piece.text.length);
    const body = piece.text.slice(start).replace(/\s+$/, "");
    if (!body) continue;
    if (sync.length) text += "\n\n";
    text += `## ${piece.heading}\n\n`;
    const base = text.length;
    for (const p of points)
      if (p.chars - start <= body.length)
        sync.push({
          chars: base + (p.chars - start),
          ms: Math.max(0, Math.round(p.ms + piece.offsetMs)),
        });
    text += body;
  }
  return { text, sync };
}
