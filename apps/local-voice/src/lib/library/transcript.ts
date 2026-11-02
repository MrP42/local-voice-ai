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
