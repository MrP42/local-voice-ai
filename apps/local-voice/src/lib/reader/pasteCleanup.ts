/**
 * Bereinigung beim Einfügen ohne KI (lva#530 I1).
 *
 * Text aus Intranet, PDF, Word oder Newslettern bringt Reste mit, die beim
 * Vorlesen stören: Seitenzahlen, wiederkehrende Kopf- und Fußzeilen,
 * Bedienelemente von Webseiten („Drucken", „Teilen"), unsichtbare Zeichen,
 * Silbentrennung am Zeilenende. Alles hier läuft nach festen Regeln und
 * kostet kein Token.
 *
 * Bewusst NICHT dabei: Zeitstempel (Agenden wie „(10:30) Begrüßung" bleiben)
 * und das Verbinden einzelner Satzzeilen zu Absätzen. Sprecher `<Name>`,
 * Tags `[…]`, Seitenmarken `{{S. n}}` und Markdown-Blöcke bleiben unberührt.
 */
import { joinFragments, tidySpaces } from "./textCleanup.ts";

export type PasteRule =
  | "control"
  | "pageNumbers"
  | "repeated"
  | "webNoise"
  | "fragments"
  | "spaces"
  | "blankLines";

/** Reihenfolge der Regeln: erst Zeichen, dann Zeilen entfernen, dann
 *  zusammenfügen, zuletzt Leerzeilen. */
export const PASTE_RULES: readonly PasteRule[] = [
  "control",
  "pageNumbers",
  "repeated",
  "webNoise",
  "fragments",
  "spaces",
  "blankLines",
];

export interface PasteRuleResult {
  text: string;
  changes: number;
}

export interface PasteItem {
  rule: PasteRule;
  count: number;
}

export interface PasteCleanResult {
  text: string;
  changes: number;
  items: PasteItem[];
}

export interface PasteCleanOptions {
  /** Nur diese Regeln anwenden (Standard: alle, in fester Reihenfolge). */
  rules?: readonly PasteRule[];
}

/** Ab so vielen Zeichen gilt ein Text als lang genug für Kopf-/Fußzeilen. */
export const REPEATED_MIN_CHARS = 1500;
/** Kurzer Einzeiler wird nie angefasst. */
const SHORT_CHARS = 80;
/** Mindestabstand (Zeilen) zwischen zwei Vorkommen einer Kopf-/Fußzeile. */
const REPEATED_MIN_GAP = 8;

/** Markdown-Block der Buchansicht (siehe `readerDoc.ts`). */
const BLOCK_START =
  /^\s*(?:#{1,3}\s|>\s?|[-*]\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$|⁂)/;
const SPEAKER_START = /^\s*<[^<>\n]{1,40}>/;
const TAG_START = /^\s*\[[^\]\n]*\]/;
const PAGE_MARK = /^\s*\{\{S\./;

/** Zeile gehört zur Struktur des Dokuments und wird nie entfernt. */
const isStructural = (line: string) =>
  BLOCK_START.test(line) ||
  SPEAKER_START.test(line) ||
  TAG_START.test(line) ||
  PAGE_MARK.test(line);

// ---------------------------------------------------------------------------
// control
// ---------------------------------------------------------------------------

/** Steuerzeichen außer Tab, Zeilenumbruch und Formfeed (der wird zum Umbruch). */
const CONTROL_CHARS = /[\u0000-\u0008\u000B\u000E-\u001F\u007F-\u009F]/g;
const SOFT_HYPHEN_BREAK = /­[ \t]*\n(?=\p{Ll})/gu;
const INVISIBLE = /[­​-‍﻿]/g;
const NBSP = / /g;

const countMatches = (text: string, re: RegExp) =>
  (text.match(re) ?? []).length;

/**
 * Zeilenenden vereinheitlichen (zählt nicht: die Textarea tut es ohnehin),
 * Formfeed zu Umbruch, Steuerzeichen, weiches Trennzeichen und Nullbreiten
 * entfernen, geschütztes Leerzeichen zu Leerzeichen.
 */
export function stripControl(text: string): PasteRuleResult {
  let changes = 0;
  let out = text.replace(/\r\n?/g, "\n");
  changes += countMatches(out, /\f/g);
  out = out.replace(/\f/g, "\n");
  changes += countMatches(out, CONTROL_CHARS);
  out = out.replace(CONTROL_CHARS, "");
  // `Mit-­` am Zeilenende vor Kleinbuchstaben: Wort wächst zusammen.
  changes += countMatches(out, SOFT_HYPHEN_BREAK);
  out = out.replace(SOFT_HYPHEN_BREAK, "");
  changes += countMatches(out, INVISIBLE);
  out = out.replace(INVISIBLE, "");
  changes += countMatches(out, NBSP);
  out = out.replace(NBSP, " ");
  return { text: out, changes };
}

// ---------------------------------------------------------------------------
// Zeilen entfernen (pageNumbers, repeated, webNoise)
// ---------------------------------------------------------------------------

/**
 * Zeilen entfernen, für die `drop(index)` gilt. Steht die entfernte Zeile
 * zwischen zwei Leerzeilen (oder am Textanfang vor einer), fällt eine davon
 * mit weg: sonst bliebe an der Stelle eine doppelte Lücke.
 */
function dropLines(
  lines: readonly string[],
  drop: (index: number) => boolean,
): PasteRuleResult {
  const out: string[] = [];
  let changes = 0;
  let skipBlank = false;
  lines.forEach((line, index) => {
    if (skipBlank && line.trim() === "") {
      skipBlank = false;
      return;
    }
    skipBlank = false;
    if (!drop(index)) {
      out.push(line);
      return;
    }
    changes += 1;
    const prevBlank = out.length === 0 || out[out.length - 1].trim() === "";
    const next = lines[index + 1];
    skipBlank = prevBlank && next !== undefined && next.trim() === "";
  });
  return { text: out.join("\n"), changes };
}

// ---------------------------------------------------------------------------
// pageNumbers
// ---------------------------------------------------------------------------

const BARE_NUMBER = /^\d{1,3}$/;
const DASHED_NUMBER = /^[-–—]\s*\d{1,3}\s*[-–—]$/;
const WORD_PAGE =
  /^(?:Seite|S\.|Page|p\.)\s*(\d{1,3})(?:\s*(?:von|of|\/)\s*(\d{1,3}))?$/i;
const SLASH_PAGE = /^(\d{1,3})\s*\/\s*(\d{1,3})$/;

/** Besteht die Zeile nur aus einer Seitenzahl? */
export function isPageNumberLine(line: string): boolean {
  const s = line.trim();
  if (BARE_NUMBER.test(s) || DASHED_NUMBER.test(s)) return true;
  const word = WORD_PAGE.exec(s);
  if (word) {
    return word[2] === undefined || Number(word[2]) >= Number(word[1]);
  }
  const slash = SLASH_PAGE.exec(s);
  return slash !== null && Number(slash[2]) >= Number(slash[1]);
}

/** Zeilen, die nur eine Seitenzahl sind („12", „- 12 -", „Seite 3 von 30"). */
export function stripPageNumbers(text: string): PasteRuleResult {
  const lines = text.split("\n");
  return dropLines(lines, (i) => isPageNumberLine(lines[i]));
}

// ---------------------------------------------------------------------------
// repeated
// ---------------------------------------------------------------------------

const SENTENCE_END = /[.!?…]["'»«“”‘’)\]]*$/;

const repeatKey = (line: string): string | null => {
  const s = line.trim().replace(/\s+/g, " ");
  if (s.length < 3 || s.length > 120) return null;
  if (isStructural(s)) return null;
  // Ein Satz mit Punkt ist Inhalt (Kehrreim, „Vielen Dank."), keine Kopfzeile.
  if (SENTENCE_END.test(s)) return null;
  // Zeile endet mit Silbentrennung: gehört zur nächsten Zeile.
  if (/\p{L}-$/u.test(s)) return null;
  return s.replace(/\d+/g, "#");
};

/**
 * Wiederkehrende Kopf- und Fußzeilen: dieselbe Zeile (Ziffern gleichgesetzt)
 * mindestens dreimal in einem langen Text, jeweils mindestens acht Zeilen
 * auseinander. Kürzere Texte und dichte Wiederholungen (Kehrreim) bleiben.
 */
export function stripRepeated(text: string): PasteRuleResult {
  if (text.length < REPEATED_MIN_CHARS) return { text, changes: 0 };
  const lines = text.split("\n");
  const seen = new Map<string, number[]>();
  lines.forEach((line, index) => {
    const key = repeatKey(line);
    if (key === null) return;
    const at = seen.get(key);
    if (at) at.push(index);
    else seen.set(key, [index]);
  });
  const dropped = new Set<number>();
  for (const at of seen.values()) {
    if (at.length < 3) continue;
    const spaced = at.every(
      (pos, i) => i === 0 || pos - at[i - 1] >= REPEATED_MIN_GAP,
    );
    if (spaced) at.forEach((pos) => dropped.add(pos));
  }
  if (dropped.size === 0) return { text, changes: 0 };
  return dropLines(lines, (i) => dropped.has(i));
}

// ---------------------------------------------------------------------------
// webNoise
// ---------------------------------------------------------------------------

const WEB_NOISE = new Set(
  [
    "Zum Inhalt springen",
    "Zur Navigation springen",
    "Drucken",
    "Seite drucken",
    "Druckversion",
    "Teilen",
    "Artikel teilen",
    "Link kopieren",
    "Nach oben",
    "Zurück",
    "Zurück zur Übersicht",
    "Mehr lesen",
    "Weiterlesen",
    "Cookie-Einstellungen",
    "Alle akzeptieren",
    "Skip to content",
    "Back to top",
    "Print",
    "Share",
    "Facebook",
    "LinkedIn",
    "Twitter",
    "X",
    "Xing",
    "WhatsApp",
    "E-Mail",
  ].map((s) => s.toLowerCase()),
);

const NOISE_SEPARATOR = /\s*[·|/–•]\s*/;

/** Besteht die Zeile nur aus Bedienresten (einzeln oder mit · | / – getrennt)? */
export function isWebNoiseLine(line: string): boolean {
  const s = line.trim();
  if (s === "") return false;
  const parts = s.split(NOISE_SEPARATOR).filter((p) => p.trim() !== "");
  if (parts.length === 0) return false;
  return parts.every((p) => WEB_NOISE.has(p.trim().toLowerCase()));
}

/** Zeilen aus Bedienelementen von Webseiten entfernen. */
export function stripWebNoise(text: string): PasteRuleResult {
  const lines = text.split("\n");
  return dropLines(lines, (i) => isWebNoiseLine(lines[i]));
}

// ---------------------------------------------------------------------------
// blankLines
// ---------------------------------------------------------------------------

/**
 * Drei und mehr Leerzeilen in Folge zu einer, Leerzeilen am Anfang und Ende
 * weg. Ein einzelner Zeilenumbruch am Ende zählt nicht als Änderung.
 */
export function collapseBlankLines(text: string): PasteRuleResult {
  const lines = text.split("\n").map((l) => (l.trim() === "" ? "" : l));
  let changes = 0;
  let start = 0;
  while (start < lines.length && lines[start] === "") start += 1;
  let end = lines.length;
  while (end > start && lines[end - 1] === "") end -= 1;
  if (start > 0) changes += 1;
  // Ein abschließender Umbruch ergibt genau eine leere letzte Zeile.
  if (lines.length - end > 1) changes += 1;
  const out: string[] = [];
  let run = 0;
  const flush = () => {
    if (run === 0) return;
    if (run >= 3) {
      out.push("");
      changes += 1;
    } else {
      for (let i = 0; i < run; i += 1) out.push("");
    }
    run = 0;
  };
  for (let i = start; i < end; i += 1) {
    if (lines[i] === "") {
      run += 1;
      continue;
    }
    flush();
    out.push(lines[i]);
  }
  return { text: out.join("\n"), changes };
}

// ---------------------------------------------------------------------------
// Gesamt
// ---------------------------------------------------------------------------

const RULE_FNS: Record<PasteRule, (text: string) => PasteRuleResult> = {
  control: stripControl,
  pageNumbers: stripPageNumbers,
  repeated: stripRepeated,
  webNoise: stripWebNoise,
  fragments: joinFragments,
  spaces: tidySpaces,
  blankLines: collapseBlankLines,
};

/** Kurzer Einzeiler: nichts zu bereinigen, nichts anfassen. */
const isShortLine = (text: string) => {
  const t = text.trim();
  return !/[\r\n\f]/.test(t) && t.length < SHORT_CHARS;
};

/** Eingefügten Text nach festen Regeln bereinigen. */
export function cleanPasted(
  text: string,
  opts: PasteCleanOptions = {},
): PasteCleanResult {
  if (isShortLine(text)) return { text, changes: 0, items: [] };
  const wanted = opts.rules ?? PASTE_RULES;
  let current = text;
  const items: PasteItem[] = [];
  for (const rule of PASTE_RULES) {
    if (!wanted.includes(rule)) continue;
    const result = RULE_FNS[rule](current);
    current = result.text;
    if (result.changes > 0) items.push({ rule, count: result.changes });
  }
  const changes = items.reduce((sum, item) => sum + item.count, 0);
  return { text: current, changes, items };
}
