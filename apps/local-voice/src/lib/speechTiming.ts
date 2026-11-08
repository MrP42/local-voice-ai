/**
 * Welche Textstelle eines Satzes gerade zu hören ist.
 *
 * Der Kern meldet mit `tts-current-sentence` die Dauer des Satzes und die
 * stimmhaften Abschnitte darin (`speech_ms`, ms bei Tempo 1). Die Dauer
 * enthält Vorlauf, Nachklang und Absatzpause; wer die Zeichen gleichmäßig auf
 * sie verteilt, markiert gegen Satzende um genau diese Stille zu spät
 * (Patrick 08.11.). Verteilt wird deshalb nur über die Sprechzeit.
 *
 * Passt die Zahl der Abschnitte zu den Teilsätzen (Komma, Semikolon,
 * Doppelpunkt, Gedankenstrich), bekommt jeder Teilsatz seinen Abschnitt:
 * die Kommapause der Stimme hält die Markierung dann am Kommawort fest,
 * statt sie weiterlaufen zu lassen. Sonst gilt die Sprechzeit als Ganzes.
 */

export type SpeechSpan = readonly [number, number];

export interface TextRange {
  start: number;
  end: number;
}

/** Hinter diesen Zeichen (gefolgt von Leerraum) macht eine Stimme eine Pause. */
const PAUSE_AFTER = /[,;:–—…]/;

/** Gültige, aufsteigende Abschnitte; Unsinn aus dem Ereignis fällt weg. */
export function cleanSpans(spans: unknown): SpeechSpan[] {
  if (!Array.isArray(spans)) return [];
  const out: SpeechSpan[] = [];
  let last = -Infinity;
  for (const span of spans) {
    if (!Array.isArray(span) || span.length < 2) return [];
    const [a, b] = span;
    if (!Number.isFinite(a) || !Number.isFinite(b) || b <= a || a < last)
      return [];
    out.push([a, b]);
    last = b;
  }
  return out;
}

/** Teilsätze von `range`: Grenze hinter einem Pausenzeichen vor Leerraum. */
export function clauses(text: string, range: TextRange): TextRange[] {
  const out: TextRange[] = [];
  let start = range.start;
  for (let i = range.start; i < range.end - 1; i++) {
    if (!PAUSE_AFTER.test(text[i]) || !/\s/.test(text[i + 1])) continue;
    const cut = i + 1;
    if (cut > start) out.push({ start, end: cut });
    // Der nächste Teilsatz beginnt mit seinem ersten Wort, nicht im Leerraum.
    start = cut;
    while (start < range.end && /\s/.test(text[start])) start++;
    i = start - 1;
  }
  if (range.end > start) out.push({ start, end: range.end });
  return out;
}

/**
 * Textoffset in `range`, der nach `elapsedMs` (Tempo 1, ab Satzbeginn) zu
 * hören ist. Vor dem ersten Laut der Satzanfang, in einer Pause das Ende des
 * zuvor gesprochenen Stücks, nach dem letzten Laut das Satzende.
 */
export function spokenOffset(
  text: string,
  range: TextRange,
  spans: readonly SpeechSpan[],
  elapsedMs: number,
): number {
  const length = range.end - range.start;
  if (length <= 0 || spans.length === 0) return range.start;
  const parts = clauses(text, range);
  const lastChar = range.end - 1;
  if (parts.length === spans.length && parts.length > 1) {
    for (let i = spans.length - 1; i >= 0; i--) {
      const [a, b] = spans[i];
      if (elapsedMs < a) continue;
      const part = parts[i];
      const share = Math.min(1, (elapsedMs - a) / (b - a));
      const at = part.start + Math.floor(share * (part.end - part.start));
      return Math.min(at, part.end - 1, lastChar);
    }
    return range.start;
  }
  let spoken = 0;
  let total = 0;
  for (const [a, b] of spans) {
    total += b - a;
    spoken += Math.max(0, Math.min(elapsedMs, b) - a);
  }
  if (total <= 0) return range.start;
  const at = range.start + Math.floor((spoken / total) * length);
  return Math.min(at, lastChar);
}
