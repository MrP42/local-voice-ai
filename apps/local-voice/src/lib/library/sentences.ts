/**
 * Sätze eines Absatzes für das Mitlesen (Hörbuch-Sync Stufe 3): Grenzen als
 * Zeichenstellen im ganzen Buchtext (UTF-16 wie JS). Mit `Intl.Segmenter`
 * wo vorhanden, sonst eine einfache Regel (Satzzeichen + Leerraum).
 */

export interface Span {
  start: number;
  end: number;
}

type Segmenter = {
  segment(s: string): Iterable<{ index: number; segment: string }>;
};

let segmenter: Segmenter | null | undefined;
const getSegmenter = (): Segmenter | null => {
  if (segmenter !== undefined) return segmenter;
  const I = Intl as unknown as {
    Segmenter?: new (l: string, o: { granularity: "sentence" }) => Segmenter;
  };
  segmenter = I.Segmenter
    ? new I.Segmenter("de", { granularity: "sentence" })
    : null;
  return segmenter;
};

/** Sätze von `text`, das bei `offset` im Buch beginnt; Leerraum am Rand fällt weg. */
export function sentencesOf(text: string, offset: number): Span[] {
  const raw: Span[] = [];
  const seg = getSegmenter();
  if (seg) {
    for (const s of seg.segment(text))
      raw.push({ start: s.index, end: s.index + s.segment.length });
  } else {
    const re = /[.!?…]+["“”»«']*\s+/g;
    let start = 0;
    for (let m = re.exec(text); m; m = re.exec(text)) {
      raw.push({ start, end: m.index + m[0].length });
      start = m.index + m[0].length;
    }
    if (start < text.length) raw.push({ start, end: text.length });
  }
  const out: Span[] = [];
  for (const r of raw) {
    let a = r.start;
    let b = r.end;
    while (a < b && /\s/.test(text[a])) a++;
    while (b > a && /\s/.test(text[b - 1])) b--;
    if (b > a) out.push({ start: offset + a, end: offset + b });
  }
  return out;
}

/** Satz, in dem `chars` liegt (sonst der letzte davor, sonst der erste). */
export function sentenceAt(sentences: Span[], chars: number): Span | null {
  if (!sentences.length) return null;
  let found = sentences[0];
  for (const s of sentences) {
    if (s.start > chars) break;
    found = s;
  }
  return found;
}
