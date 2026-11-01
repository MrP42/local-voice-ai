/**
 * Findet den Satz, den die Engine gerade spricht, im Seitentext.
 *
 * Das Backend meldet mit `tts-current-sentence` Index und Text des Satzes,
 * aber keine Position: die Sätze entstehen dort aus dem Sprechertext (ohne
 * Marker, nach `split_sentences`). Statt die Satzgrenzen hier ein zweites Mal
 * nachzubauen, sucht der Locator den gemeldeten Text in den sichtbaren
 * Läufen des Lesedokuments — vorwärts ab dem letzten Treffer, damit ein Satz,
 * der zweimal vorkommt („Ja.“), an der richtigen Stelle markiert wird.
 * Verglichen wird normalisiert: ohne Marker, Tags, Markdown, mit
 * zusammengefassten Leerzeichen.
 */
import type { ReaderBlock } from "./readerDoc";

export interface SourceRange {
  start: number;
  end: number;
}

interface Stream {
  text: string;
  /** Für jedes Zeichen in `text` der Offset im Seitentext. */
  map: number[];
  /** Endoffset (exklusiv) jedes Zeichens. */
  mapEnd: number[];
}

/** Kleinschreibung je Zeichen; Faltungen, die die Länge ändern („İ“), bleiben
 *  unverändert, damit Strom und Offset-Karte gleich lang bleiben. */
const fold = (c: string) => {
  const f = c.toLocaleLowerCase("de");
  return f.length === c.length ? f : c;
};

/** Sichtbaren Text aller Blöcke als normalisierten Strom mit Offsets bauen. */
export function buildStream(blocks: ReaderBlock[]): Stream {
  let text = "";
  const map: number[] = [];
  const mapEnd: number[] = [];
  let lastSpace = true;
  const push = (c: string, at: number, end: number) => {
    text += c;
    map.push(at);
    mapEnd.push(end);
  };
  for (const block of blocks) {
    for (const run of block.runs) {
      if (run.tag) continue;
      for (let i = 0; i < run.text.length; i++) {
        const c = run.text[i];
        if (/\s/.test(c)) {
          if (!lastSpace) push(" ", run.start + i, run.start + i + 1);
          lastSpace = true;
        } else {
          push(fold(c), run.start + i, run.start + i + 1);
          lastSpace = false;
        }
      }
    }
    if (!lastSpace) push(" ", block.end, block.end);
    lastSpace = true;
  }
  return { text, map, mapEnd };
}

/** Den gemeldeten Satz so normalisieren wie den Strom. */
export function normalizeSentence(sentence: string): string {
  return [
    ...sentence
      .replace(/<[^<>\n]{1,60}>/g, " ")
      .replace(/\[[^\]\n]*\]/g, " ")
      .replace(/\*\*|(^|\s)[*_]|[*_](?=\s|$)/g, "$1")
      .replace(/\s+/g, " ")
      .trim(),
  ]
    .map(fold)
    .join("");
}

export class SentenceLocator {
  private stream: Stream;
  private cursor = 0;

  constructor(blocks: ReaderBlock[]) {
    this.stream = buildStream(blocks);
  }

  /** Nach einem Sprung (Satz zurück, Klick in den Text) neu ansetzen. */
  reset(sourceOffset = 0): void {
    const i = this.stream.map.findIndex((o) => o >= sourceOffset);
    this.cursor = i === -1 ? 0 : i;
  }

  locate(sentence: string): SourceRange | null {
    let needle = normalizeSentence(sentence);
    if (!needle) return null;
    const hay = this.stream.text;
    let at = hay.indexOf(needle, this.cursor);
    if (at === -1) at = hay.indexOf(needle);
    if (at === -1 && /[.!?…]$/.test(needle)) {
      // Überschriften bekommen im Sprechtext einen Punkt, den die Seite nicht zeigt.
      const bare = needle.replace(/[.!?…]+$/, "");
      at = hay.indexOf(bare, this.cursor);
      if (at === -1) at = hay.indexOf(bare);
      if (at !== -1) needle = bare;
    }
    let length = needle.length;
    if (at === -1) {
      // Engine-Text weicht ab (z. B. abgeschnittener Satz am Textende): die
      // ersten Wörter reichen, um die Stelle zu finden.
      const head = needle.slice(0, Math.min(needle.length, 32));
      at = hay.indexOf(head, this.cursor);
      if (at === -1) at = hay.indexOf(head);
      if (at === -1) return null;
      length = Math.min(needle.length, hay.length - at);
    }
    const last = Math.min(at + length, hay.length) - 1;
    this.cursor = last + 1;
    return { start: this.stream.map[at], end: this.stream.mapEnd[last] };
  }
}
