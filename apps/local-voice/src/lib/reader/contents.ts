import type { ReaderBlock } from "./readerDoc";

/** Eintrag im Inhaltsverzeichnis: eine Überschrift des Seitentexts. */
export interface ContentsEntry {
  title: string;
  level: number;
  start: number;
}

/** Überschriften (# bis ###) als Inhaltsverzeichnis, ohne Tags. */
export function buildContents(blocks: ReaderBlock[]): ContentsEntry[] {
  const out: ContentsEntry[] = [];
  for (const b of blocks) {
    if (b.kind !== "heading") continue;
    const title = b.runs
      .filter((r) => !r.tag)
      .map((r) => r.text)
      .join("")
      .replace(/\s+/g, " ")
      .trim();
    // Sprungziel ist der erste sichtbare Text, nicht das „#“ davor.
    const first = b.runs.find((r) => !r.tag);
    if (title && first)
      out.push({ title, level: b.level ?? 1, start: first.start });
  }
  return out;
}

/** Index des Kapitels, in dem `offset` liegt (letzte Überschrift davor), oder -1. */
export function chapterAt(contents: ContentsEntry[], offset: number): number {
  let found = -1;
  for (let i = 0; i < contents.length; i++) {
    if (contents[i].start <= offset) found = i;
    else break;
  }
  return found;
}
