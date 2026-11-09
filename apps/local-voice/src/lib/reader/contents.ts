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

/** Titelei eines importierten Buchs: `# Titel` und darunter `von Autor, …`. */
export interface BookInfo {
  title: string;
  authors: string;
}

/**
 * Titel und Autor aus der Titelseite, die der EPUB-Import voranstellt (lva#149).
 * Nur wenn beides da ist — ein Text, der bloß mit einer Überschrift beginnt,
 * ist kein Buch mit Titelei.
 */
export function bookInfo(blocks: ReaderBlock[]): BookInfo | null {
  const [head, by] = blocks;
  if (head?.kind !== "heading" || (head.level ?? 1) !== 1) return null;
  if (by?.kind !== "paragraph") return null;
  const text = (b: ReaderBlock) =>
    b.runs
      .filter((r) => !r.tag)
      .map((r) => r.text)
      .join("")
      .replace(/\s+/g, " ")
      .trim();
  // Eine Autorzeile, kein Satz: „von dort aus sah man das Meer.“ ist keine,
  // Initialen („von A. B. Müller“) schon (wie TITLE_PAGE in isbn.ts).
  const authors = /^von (\p{Lu}(?:[^!?…]{0,200}[^.!?…:;,])?)$/u
    .exec(text(by))?.[1]
    ?.trim();
  const title = text(head);
  return title && authors ? { title, authors } : null;
}
