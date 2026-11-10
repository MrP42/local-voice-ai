/**
 * Volltextsuche im offenen Buch (lva#476, Patrick 10.11.: „es gibt auch keine
 * Schlagwortsuche in Büchern“).
 *
 * Gesucht wird im sichtbaren Lesetext (ohne Sprecher-Marker, Tags und
 * Seitenmarken), gefaltet wie die Volltextsuche der Ablage: klein, ohne
 * Diakritika (ä → a, é → e) und mit der Ersatztabelle `FTS_FALTUNG` aus
 * wai-retrieval (ß → ss, æ → ae, ø → o …). „strasse“ findet „Straße“,
 * „Marchen“ findet „Märchen“. Leerraum zählt als ein Leerzeichen, auch über
 * Absatzgrenzen nicht hinweg (jeder Block endet mit einem Leerzeichen).
 *
 * Jeder Treffer trägt seine Quell-Offsets im Seitentext, damit die Buchansicht
 * dorthin blättern und ihn markieren kann, und einen kurzen Ausschnitt aus dem
 * sichtbaren Text für die Trefferliste.
 */
import type { ReaderBlock } from "./readerDoc";

/** Wie `FTS_FALTUNG` in wai-retrieval (`fts_normalisieren`). */
const FALTUNG: Record<string, string> = {
  ß: "ss",
  ẞ: "ss",
  æ: "ae",
  Æ: "ae",
  œ: "oe",
  Œ: "oe",
  ø: "o",
  Ø: "o",
  ł: "l",
  Ł: "l",
  đ: "d",
  Đ: "d",
  ð: "d",
  Ð: "d",
  þ: "th",
  Þ: "th",
  ħ: "h",
  Ħ: "h",
  ı: "i",
  ĳ: "ij",
  Ĳ: "ij",
};

const MARKS = /\p{M}/gu;
/** Unsichtbare Zeichen, die beim Suchen nicht zählen (weiches Trennzeichen, Nullbreite). */
const INVISIBLE = /[­​-‍⁠﻿]/;

/** Ein Zeichen falten; das Ergebnis kann leer oder länger sein. */
export function foldChar(c: string): string {
  if (INVISIBLE.test(c)) return "";
  const table = FALTUNG[c];
  if (table) return table;
  const plain = c.normalize("NFD").replace(MARKS, "");
  return plain.toLocaleLowerCase("de").replace(MARKS, "");
}

/** Text falten wie den Suchstrom (für Anfragen und Tests). */
export function foldText(text: string): string {
  let out = "";
  let space = true;
  for (const c of text) {
    if (/\s/.test(c)) {
      if (!space) out += " ";
      space = true;
      continue;
    }
    const f = foldChar(c);
    if (!f) continue;
    out += f;
    space = false;
  }
  return out.trimEnd();
}

export interface SearchIndex {
  /** Gefalteter Text. */
  folded: string;
  /** Je Zeichen in `folded`: Index im sichtbaren Text `shown`. */
  shownAt: number[];
  /** Sichtbarer Text, Leerraum zusammengefasst. */
  shown: string;
  /** Je Zeichen in `shown`: Start- und Endoffset im Seitentext. */
  start: number[];
  end: number[];
  /** Je Zeichen in `shown`: Index des Blocks. */
  block: number[];
}

/** Den Suchstrom aus den Blöcken der Leseansicht bauen (einmal je Text). */
export function buildSearchIndex(blocks: ReaderBlock[]): SearchIndex {
  // Teile sammeln statt `+=` und je Zeichen gefaltet nur einmal (Cache):
  // ein Buch mit 1,2 Mio. Zeichen braucht so rund 0,25 s statt 0,6 s.
  const foldedParts: string[] = [];
  const shownParts: string[] = [];
  const shownAt: number[] = [];
  const start: number[] = [];
  const end: number[] = [];
  const block: number[] = [];
  const cache = new Map<string, string>();
  let shownLen = 0;
  let lastSpace = true;
  const space = (s: number, e: number, bi: number) => {
    shownParts.push(" ");
    start.push(s);
    end.push(e);
    block.push(bi);
    shownAt.push(shownLen++);
    foldedParts.push(" ");
    lastSpace = true;
  };
  blocks.forEach((b, bi) => {
    for (const run of b.runs) {
      if (run.tag) continue;
      const text = run.text;
      for (let i = 0; i < text.length;) {
        const code = text.codePointAt(i) ?? 0;
        const n = code > 0xffff ? 2 : 1;
        const at = run.start + i;
        i += n;
        let f: string;
        if (code < 128) {
          // ASCII: Leerraum oder Kleinbuchstabe, ohne Regex und Normalisierung.
          if (code === 32 || (code >= 9 && code <= 13)) {
            if (!lastSpace) space(at, at + 1, bi);
            continue;
          }
          f =
            code >= 65 && code <= 90
              ? String.fromCharCode(code + 32)
              : text[at - run.start];
        } else {
          const c = text.slice(i - n, i);
          if (/\s/.test(c)) {
            if (!lastSpace) space(at, at + n, bi);
            continue;
          }
          let cached = cache.get(c);
          if (cached === undefined) {
            cached = foldChar(c);
            cache.set(c, cached);
          }
          f = cached;
          if (!f) continue;
        }
        shownParts.push(text.slice(i - n, i));
        // `shown` zählt in UTF-16-Einheiten wie `String.slice`.
        for (let k = 0; k < n; k++) {
          start.push(at);
          end.push(at + n);
          block.push(bi);
        }
        for (let k = 0; k < f.length; k++) shownAt.push(shownLen);
        shownLen += n;
        foldedParts.push(f);
        lastSpace = false;
      }
    }
    if (!lastSpace) space(b.end, b.end, bi);
  });
  return {
    folded: foldedParts.join(""),
    shownAt,
    shown: shownParts.join(""),
    start,
    end,
    block,
  };
}

export interface SearchHit {
  /** Offsets im Seitentext (für Blättern und Markierung). */
  start: number;
  end: number;
  /** Ausschnitt: Text davor, Treffer, Text danach (sichtbarer Text). */
  before: string;
  match: string;
  after: string;
}

export interface SearchResult {
  hits: SearchHit[];
  /** Alle Treffer, auch die über `limit` hinaus. */
  total: number;
  /** Treffer, weil alle Wörter im selben Absatz stehen (nicht als Wortfolge). */
  allWords: boolean;
}

/** Kürzeste Anfrage, ab der gesucht wird (gefaltet). */
export const MIN_QUERY = 2;
const CONTEXT = 48;

function hitAt(index: SearchIndex, from: number, to: number): SearchHit {
  // Ein Treffer, der mitten in einer Faltung endet („ß“ ↔ „s“), umfasst das
  // ganze Zeichen.
  const s0 = index.shownAt[from];
  let s1 = index.shownAt[to - 1] + 1;
  // Nie zwischen den Hälften eines Ersatzpaars (Emoji) schneiden.
  if (/[\udc00-\udfff]/.test(index.shown[s1] ?? "")) s1++;
  let b = Math.max(0, s0 - CONTEXT);
  let a = Math.min(index.shown.length, s1 + CONTEXT);
  // Ausschnitt an Wortgrenzen.
  if (b > 0) {
    const sp = index.shown.indexOf(" ", b);
    if (sp !== -1 && sp < s0) b = sp + 1;
  }
  if (a < index.shown.length) {
    const sp = index.shown.lastIndexOf(" ", a);
    if (sp > s1) a = sp;
  }
  return {
    start: index.start[s0],
    end: index.end[s1 - 1],
    before: (b > 0 ? "…" : "") + index.shown.slice(b, s0).trimStart(),
    match: index.shown.slice(s0, s1),
    after:
      index.shown.slice(s1, a).trimEnd() + (a < index.shown.length ? "…" : ""),
  };
}

/**
 * Im Buch suchen: zuerst die Anfrage als Wortfolge. Findet die nichts und hat
 * die Anfrage mehrere Wörter, gilt ein Absatz als Treffer, in dem alle Wörter
 * vorkommen (markiert wird das erste).
 */
export function searchBook(
  index: SearchIndex,
  query: string,
  limit = 300,
): SearchResult {
  const needle = foldText(query);
  const empty: SearchResult = { hits: [], total: 0, allWords: false };
  if (needle.length < MIN_QUERY) return empty;
  const hay = index.folded;
  const hits: SearchHit[] = [];
  let total = 0;
  for (
    let at = hay.indexOf(needle);
    at !== -1;
    at = hay.indexOf(needle, at + needle.length)
  ) {
    total++;
    if (hits.length < limit) hits.push(hitAt(index, at, at + needle.length));
  }
  if (total > 0) return { hits, total, allWords: false };

  const words = [
    ...new Set(needle.split(" ").filter((w) => w.length >= MIN_QUERY)),
  ];
  if (words.length < 2) return empty;
  // Je Block die gefalteten Grenzen bestimmen.
  const blockOf = (i: number) => index.block[index.shownAt[i]];
  let i = 0;
  while (i < hay.length) {
    const b = blockOf(i);
    let j = i;
    while (j < hay.length && blockOf(j) === b) j++;
    const part = hay.slice(i, j);
    if (words.every((w) => part.includes(w))) {
      total++;
      if (hits.length < limit) {
        const first = Math.min(...words.map((w) => part.indexOf(w)));
        const w = words.find((x) => part.indexOf(x) === first) ?? words[0];
        hits.push(hitAt(index, i + first, i + first + w.length));
      }
    }
    i = j;
  }
  return { hits, total, allWords: total > 0 };
}
