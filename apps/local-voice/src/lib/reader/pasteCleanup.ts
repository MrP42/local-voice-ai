/**
 * Bereinigung beim Einfügen ohne KI (lva#530 I1).
 *
 * Text aus Intranet, PDF, Word oder Newslettern bringt Reste mit, die beim
 * Vorlesen stören: Seitenzahlen, wiederkehrende Kopf- und Fußzeilen,
 * Bedienelemente von Webseiten („Drucken", „Teilen"), unsichtbare Zeichen,
 * Silbentrennung am Zeilenende. Alles hier läuft nach festen Regeln und
 * kostet kein Token. Keine Regel durchsucht ganze Zeilen mit rückverfolgenden
 * Mustern (Zeilen werden gesplittet, Zeichenklassen-Muster haben feste Länge);
 * der Aufwand wächst mit O(n log n) (Sortierung der Zeilenabstände einer
 * Kopfzeile), die Durchläufe sind auf wenige begrenzt.
 *
 * Bewusst NICHT dabei: Zeitstempel (Agenden wie „(10:30) Begrüßung" bleiben)
 * und das Verbinden einzelner Satzzeilen zu Absätzen.
 *
 * Geschützt vor allen Regeln, die Zeilen entfernen oder verbinden: Code
 * (umzäunte Blöcke mit ``` oder ~~~ bis zum Ende, eingerückte Blöcke) und
 * Markdown-Blockzeilen (Überschriften, Zitate, Listen, Tabellenzeilen),
 * Sprecher `<Name>`, Tags `[…]` und Seitenmarken `{{S. n}}`. In Code laufen
 * zusätzlich die Zeichen-, die Leerzeichen- und die Leerzeilenregel nicht:
 * ein Codeblock bleibt Zeichen für Zeichen, wie er eingefügt wurde.
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

/** Reihenfolge der Regeln: Kopf-/Fußzeilen zuerst (sie brauchen die Seitengrenzen,
 *  also Seitenzahlzeilen und Formfeeds, die die späteren Regeln entfernen),
 *  dann Zeichen, Zeilen entfernen, zusammenfügen, zuletzt Leerzeilen. */
export const PASTE_RULES: readonly PasteRule[] = [
  "repeated",
  "control",
  "pageNumbers",
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
/** Anteil der Seiten, auf denen eine Kopf-/Fußzeile stehen muss. */
const REPEATED_PAGE_SHARE = 0.6;
/** Zulässige Abweichung vom mittleren Abstand bei gleichmäßigem Rhythmus. */
const REPEATED_RHYTHM_TOLERANCE = 0.3;
/** Mehr Durchläufe braucht keine Vorlage; schützt vor Endlosschleifen. */
const MAX_PASSES = 6;

// ---------------------------------------------------------------------------
// Zeilenarten: Code und Struktur
// ---------------------------------------------------------------------------

/** Markdown-Block der Buchansicht (siehe `readerDoc.ts`) samt Listen. */
const BLOCK_START =
  /^\s*(?:#{1,6}\s|>\s?|[-*+]\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$|⁂|\d{1,3}[.)]\s)/;
const SPEAKER_START = /^\s*<[^<>\n]{1,40}>/;
const TAG_START = /^\s*\[[^\]\n]*\]/;
const PAGE_MARK = /^\s*\{\{S\./;
const FENCE = /^ {0,3}(`{3,}|~{3,})(.*)$/;
const INDENTED = /^(?: {4}|\t)/;

/** Tabellenzeile: beginnt oder endet mit einem senkrechten Strich. */
const isTableRow = (line: string) => {
  const s = line.trim();
  return s.startsWith("|") || s.endsWith("|");
};

/** Zeile gehört zur Struktur des Dokuments und wird nie entfernt oder verbunden. */
const isStructural = (line: string) =>
  BLOCK_START.test(line) ||
  SPEAKER_START.test(line) ||
  TAG_START.test(line) ||
  PAGE_MARK.test(line) ||
  isTableRow(line);

/**
 * code: umzäunter oder eingerückter Block, bleibt unberührt;
 * block: Markdown-Blockzeile, Sprecher, Tag, Seitenmarke;
 * blank: Leerzeile außerhalb von Code; text: Fließtext.
 */
type Kind = "code" | "block" | "text" | "blank";

/**
 * Zustandsautomat über die Zeilen. Ein umzäunter Block beginnt mit ``` oder
 * ~~~ (bis drei Leerzeichen Einrückung) und endet mit einem Zaun derselben Art
 * und mindestens derselben Länge, sonst am Textende. Ein eingerückter Block
 * (vier Leerzeichen oder Tab) beginnt nach einer Leerzeile oder am Textanfang
 * und läuft über Leerzeilen hinweg weiter, solange danach wieder eingerückt
 * wird. Eine eingerückte reine Seitenzahl („      12") eröffnet keinen Block.
 */
function classifyLines(lines: readonly string[]): Kind[] {
  const kinds: Kind[] = new Array(lines.length);
  let fence: { char: string; length: number } | null = null;
  let prevIndented = false;
  let blankStart = -1;
  let blanksAfterIndented = false;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (fence) {
      kinds[i] = "code";
      const close = FENCE.exec(line);
      if (
        close &&
        close[1][0] === fence.char &&
        close[1].length >= fence.length &&
        close[2].trim() === ""
      )
        fence = null;
      continue;
    }
    if (line.trim() === "") {
      if (blankStart < 0) {
        blankStart = i;
        blanksAfterIndented = prevIndented;
      }
      kinds[i] = "blank";
      prevIndented = false;
      continue;
    }
    const startsBlock = blankStart >= 0 || i === 0;
    const open = FENCE.exec(line);
    if (open && !(open[1][0] === "`" && open[2].includes("`"))) {
      fence = { char: open[1][0], length: open[1].length };
      kinds[i] = "code";
      blankStart = -1;
      prevIndented = false;
      continue;
    }
    if (
      INDENTED.test(line) &&
      (prevIndented || (startsBlock && !isPageNumberLine(line)))
    ) {
      if (blankStart >= 0 && blanksAfterIndented) {
        for (let j = blankStart; j < i; j += 1) kinds[j] = "code";
      }
      kinds[i] = "code";
      prevIndented = true;
      blankStart = -1;
      continue;
    }
    kinds[i] = isStructural(line) ? "block" : "text";
    prevIndented = false;
    blankStart = -1;
  }
  return kinds;
}

// ---------------------------------------------------------------------------
// control
// ---------------------------------------------------------------------------

/** Steuerzeichen außer Tab, Zeilenumbruch und Formfeed (der wird zum Umbruch). */
const CONTROL_CHARS = /[\u0000-\u0008\u000B\u000E-\u001F\u007F-\u009F]/g;
const SOFT_HYPHEN_BREAK = /\u00AD[ \t]*\n(?=\p{Ll})/gu;
/** Weiches Trennzeichen, Nullbreiten-Leerzeichen, BOM / unsichtbares Leerzeichen
 *  (U+00AD, U+200B, U+FEFF). Nullbreiten-(Nicht-)Verbinder U+200C/U+200D bleiben:
 *  sie halten Emoji-Folgen („Frau + Laptop") und persische Wörter zusammen. */
const INVISIBLE = /[\u00AD\u200B\uFEFF]/g;
const NBSP = /\u00A0/g;

/** Ersetzt alle Treffer und liefert die Anzahl in einem Durchlauf. */
function replaceCounting(
  text: string,
  re: RegExp,
  replacement: string,
): PasteRuleResult {
  let changes = 0;
  const out = text.replace(re, () => {
    changes += 1;
    return replacement;
  });
  return { text: out, changes };
}

/** Zeichenregeln auf einen zusammenhängenden Lauf ungeschützter Zeilen. */
function stripControlRun(text: string): PasteRuleResult {
  let changes = 0;
  let out = text;
  for (const [re, replacement] of [
    [CONTROL_CHARS, ""],
    // `Mit-­` am Zeilenende vor Kleinbuchstaben: Wort wächst zusammen.
    [SOFT_HYPHEN_BREAK, ""],
    [INVISIBLE, ""],
    [NBSP, " "],
  ] as const) {
    const step = replaceCounting(out, re, replacement);
    out = step.text;
    changes += step.changes;
  }
  return { text: out, changes };
}

/**
 * Zeilenenden vereinheitlichen und Formfeed zu Umbruch (Seitenwechsel, nicht
 * Inhalt; die Zeilenenden zählen nicht: die Textarea tut es ohnehin), BOM am
 * Textanfang weg. Danach, nur außerhalb von Code: Steuerzeichen, weiches
 * Trennzeichen und Nullbreiten (außer Verbindern) entfernen, geschütztes
 * Leerzeichen zu Leerzeichen. In Code bleibt jedes Zeichen unverändert.
 */
export function stripControl(text: string): PasteRuleResult {
  let out = text.replace(/\r\n?/g, "\n");
  const feeds = replaceCounting(out, /\f/g, "\n");
  out = feeds.text;
  let changes = feeds.changes;
  if (out.charCodeAt(0) === 0xfeff) {
    out = out.slice(1);
    changes += 1;
  }
  const body = mapRuns(out, (kind) => kind === "code", stripControlRun);
  return { text: body.text, changes: changes + body.changes };
}

// ---------------------------------------------------------------------------
// Zeilen entfernen (pageNumbers, repeated, webNoise)
// ---------------------------------------------------------------------------

/**
 * Zeilen entfernen, für die `drop(index)` gilt. Steht die entfernte Zeile
 * zwischen zwei Leerzeilen (oder am Textanfang vor einer), fällt eine davon
 * mit weg: sonst bliebe an der Stelle eine doppelte Lücke. Code wird nie
 * angefasst.
 */
function dropLines(
  lines: readonly string[],
  kinds: readonly Kind[],
  drop: (index: number) => boolean,
  /** Ersatz für eine entfernte Zeile (z. B. ihr Formfeed), sonst nichts. */
  leave?: (index: number) => string | null,
): PasteRuleResult {
  const out: string[] = [];
  let changes = 0;
  let skipBlank = false;
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (skipBlank && kinds[index] === "blank") {
      skipBlank = false;
      continue;
    }
    skipBlank = false;
    if (kinds[index] === "code" || !drop(index)) {
      out.push(line);
      continue;
    }
    changes += 1;
    const left = leave?.(index) ?? null;
    if (left !== null) {
      out.push(left);
      continue;
    }
    const prevBlank = out.length === 0 || out[out.length - 1].trim() === "";
    skipBlank = prevBlank && kinds[index + 1] === "blank";
  }
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
  if (s.length > 40) return false;
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
  const kinds = classifyLines(lines);
  return dropLines(lines, kinds, (i) => isPageNumberLine(lines[i]));
}

// ---------------------------------------------------------------------------
// repeated
// ---------------------------------------------------------------------------

const SENTENCE_END = /[.!?…]["'»«“”‘’)\]]*$/;
/** Seitenzahl-Muster innerhalb einer Zeile: „Seite 3", „S. 3", „3 von 30", „3 / 30". */
const PAGE_IN_LINE =
  /(?<![\p{L}\d])(?:(?:Seite|S\.|Page|p\.)\s*\d{1,3}(?!\d)|\d{1,3}\s*(?:von|of|\/)\s*\d{1,3}(?!\d))/iu;
/** Überschrift oder Kapitelzeile: Ziffern unterscheiden Zeilen, nie gleichsetzen. */
const HEADING_LIKE =
  /^(?:(?:Kapitel|Teil|Abschnitt|Artikel|Anlage|Anhang|Chapter|Part|Section|Lektion|Folie|Tag)\s+\d|§\s*\d|(?!\d{1,3}\s*(?:von|of)\s*\d)\d+(?:\.\d+)*[.)]?\s+\p{L})/iu;
/** Längere Zeilen sind nie Kopf- oder Fußzeile (vor dem Zusammenfassen der Leerzeichen). */
const REPEAT_RAW_MAX_CHARS = 5000;
const REPEAT_KEY_MAX_CHARS = 120;
/** So viele Zeilen am Anfang und Ende einer Seite gelten als Seitenrand. */
const PAGE_EDGE_LINES = 2;
/** Zeichen, die die Zeichenregel später ändert: im Schlüssel schon gleichgesetzt. */
const KEY_INVISIBLE = /[\u00AD\u200B\uFEFF\u000C]/g;
const KEY_NBSP = /\u00A0/g;

/**
 * Schlüssel einer Kopf-/Fußzeilen-Kandidatin. Leerzeichen sind zusammengefasst
 * und unsichtbare Zeichen entfernt (so, wie die späteren Regeln sie ohnehin
 * behandeln). Ziffern werden nur gleichgesetzt, wenn die Zeile selbst eine
 * Seitenzahl trägt („Amt – Seite 3 von 30") und keine Überschrift ist; sonst
 * muss die Zeile exakt wiederkehren.
 */
const repeatKey = (line: string): string | null => {
  if (line.length > REPEAT_RAW_MAX_CHARS) return null;
  const s = line
    .replace(KEY_INVISIBLE, "")
    .replace(KEY_NBSP, " ")
    .trim()
    .replace(/\s+/g, " ");
  if (s.length < 3 || s.length > REPEAT_KEY_MAX_CHARS) return null;
  if (isStructural(s) || isPageNumberLine(s)) return null;
  // Ein Satz mit Punkt ist Inhalt (Kehrreim, „Vielen Dank."), keine Kopfzeile.
  if (SENTENCE_END.test(s)) return null;
  // Zeile endet mit Silbentrennung: gehört zur nächsten Zeile.
  if (/\p{L}-$/u.test(s)) return null;
  if (PAGE_IN_LINE.test(s) && !HEADING_LIKE.test(s)) {
    return s.replace(/\d+/g, "#");
  }
  return s;
};

interface PageLayout {
  /** Seite (Abschnitt zwischen zwei Seitengrenzen) jeder Zeile. */
  page: number[];
  /** Steht die Zeile unter den ersten oder letzten Zeilen ihrer Seite? */
  edge: boolean[];
  /** Anzahl der Seiten mit Inhalt. */
  pages: number;
}

const LEADING_FEEDS = /^[ \t]*(\f+)/;

/**
 * Seitengrenzen: Zeilen, die nur eine Seitenzahl sind (außerhalb von Code),
 * und Formfeeds (am Zeilenanfang: neue Seite beginnt hier, sonst danach).
 * Eine Seitenzahlzeile gehört nicht zum Inhalt; die ersten und letzten
 * {@link PAGE_EDGE_LINES} nicht leeren Zeilen jeder Seite sind der Seitenrand,
 * egal ob die Seitenzahl oben oder unten steht.
 */
function pageLayout(
  lines: readonly string[],
  kinds: readonly Kind[],
): PageLayout {
  const page: number[] = new Array(lines.length);
  const content: number[][] = [[]];
  let current = 0;
  const open = (to: number) => {
    current = to;
    while (content.length <= current) content.push([]);
  };
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    let feeds = 0;
    let leading = 0;
    if (line.includes("\f")) {
      for (
        let at = line.indexOf("\f");
        at >= 0;
        at = line.indexOf("\f", at + 1)
      )
        feeds += 1;
      leading = LEADING_FEEDS.exec(line)?.[1].length ?? 0;
    }
    if (leading > 0) open(current + leading);
    page[i] = current;
    const bare = feeds > 0 ? line.replace(/\f/g, "") : line;
    if (kinds[i] !== "code" && isPageNumberLine(bare)) open(current + 1);
    else if (bare.trim() !== "") content[current].push(i);
    if (feeds > leading) open(current + feeds - leading);
  }
  const edge: boolean[] = new Array(lines.length).fill(false);
  let pages = 0;
  for (const rows of content) {
    if (rows.length === 0) continue;
    pages += 1;
    rows.forEach((row, rank) => {
      edge[row] =
        rank < PAGE_EDGE_LINES || rank >= rows.length - PAGE_EDGE_LINES;
    });
  }
  return { page, edge, pages };
}

/** Abstände (Zeilen) gleichmäßig: jede Lücke höchstens 30 % neben der mittleren. */
function evenlySpaced(at: readonly number[]): boolean {
  const gaps = at.slice(1).map((pos, i) => pos - at[i]);
  const sorted = [...gaps].sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  return gaps.every(
    (gap) => Math.abs(gap - median) <= median * REPEATED_RHYTHM_TOLERANCE,
  );
}

/**
 * Wiederkehrende Kopf- und Fußzeilen: dieselbe Zeile mindestens dreimal in
 * einem langen Text, jeweils mindestens acht Zeilen auseinander, und zwar
 * entweder auf mindestens 60 % der Seiten (gezählt werden die verschiedenen
 * Seiten, auf denen die Zeile steht, nicht ihre Vorkommen) oder mindestens
 * viermal in gleichmäßigem Abstand (±30 %), jedes Mal am Seitenrand. Seiten
 * kennt die Regel nur an Seitenzahlzeilen und Formfeeds; ohne erkennbare
 * Seitengrenzen (mindestens zwei Seiten) bleibt jede Zeile stehen. Kürzere
 * Texte, dichte Wiederholungen und Kehrreime bleiben ebenfalls.
 */
export function stripRepeated(text: string): PasteRuleResult {
  if (text.length < REPEATED_MIN_CHARS) return { text, changes: 0 };
  const lines = text.split("\n");
  const kinds = classifyLines(lines);
  const layout = pageLayout(lines, kinds);
  if (layout.pages < 2) return { text, changes: 0 };
  const seen = new Map<string, number[]>();
  lines.forEach((line, index) => {
    if (kinds[index] !== "text") return;
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
    if (!spaced) continue;
    const onPages =
      new Set(at.map((i) => layout.page[i])).size >=
      Math.ceil(layout.pages * REPEATED_PAGE_SHARE);
    const rhythmic =
      at.length >= 4 && at.every((i) => layout.edge[i]) && evenlySpaced(at);
    if (onPages || rhythmic) at.forEach((pos) => dropped.add(pos));
  }
  if (dropped.size === 0) return { text, changes: 0 };
  // Ein Formfeed der entfernten Zeile bleibt als Seitenwechsel stehen.
  return dropLines(
    lines,
    kinds,
    (i) => dropped.has(i),
    (i) => (lines[i].includes("\f") ? "\f" : null),
  );
}

// ---------------------------------------------------------------------------
// webNoise
// ---------------------------------------------------------------------------

const lowered = (words: readonly string[]) =>
  new Set(words.map((s) => s.toLowerCase()));

/** Eindeutige Navigationsphrasen: auch allein in einer Zeile ein Bedienrest. */
const NOISE_PHRASES = lowered([
  "Zum Inhalt springen",
  "Zur Navigation springen",
  "Skip to content",
  "Nach oben",
  "Back to top",
  "Seite drucken",
  "Druckversion",
  "Artikel teilen",
  "Link kopieren",
  "Cookie-Einstellungen",
  "Alle akzeptieren",
  "Zurück zur Übersicht",
]);

/** Einzelne Bedienwörter: nur im Kontext (Block oder Trennzeichen) ein Rest. */
const NOISE_WORDS = lowered([
  "Drucken",
  "Teilen",
  "Zurück",
  "Mehr lesen",
  "Weiterlesen",
  "Print",
  "Share",
  "Facebook",
  "LinkedIn",
  "Twitter",
  "X",
  "Xing",
  "WhatsApp",
  "E-Mail",
]);

/** Ein einzelnes Zeichen: kein rückverfolgendes Muster, die Zeile wird gesplittet. */
const NOISE_SEPARATOR = /[·|/–•]/;
const NOISE_MAX_CHARS = 200;

export type WebNoiseKind = "phrase" | "multi" | "word";

/**
 * Art des Bedienrests einer Zeile: `phrase` (eindeutige Phrase), `multi`
 * (mindestens zwei Bedienwörter mit · | / – getrennt), `word` (ein einzelnes
 * Bedienwort) oder `null` (kein Bedienrest, ganze Zeile muss passen).
 */
export function webNoiseKind(line: string): WebNoiseKind | null {
  const s = line.trim();
  if (s === "" || s.length > NOISE_MAX_CHARS) return null;
  const keys = s
    .split(NOISE_SEPARATOR)
    .map((part) => part.trim().toLowerCase())
    .filter((part) => part !== "");
  if (keys.length === 0) return null;
  if (!keys.every((k) => NOISE_PHRASES.has(k) || NOISE_WORDS.has(k)))
    return null;
  if (keys.length >= 2) return "multi";
  return NOISE_PHRASES.has(keys[0]) ? "phrase" : "word";
}

/** Besteht die Zeile nur aus Bedienresten (einzeln oder mit · | / – getrennt)? */
export const isWebNoiseLine = (line: string): boolean =>
  webNoiseKind(line) !== null;

/**
 * Zeilen aus Bedienelementen von Webseiten entfernen, nur ganze Zeilen und nur
 * im Kontext: eindeutige Phrasen und Zeilen aus mindestens zwei Bedienwörtern
 * immer, ein einzelnes Bedienwort („E-Mail", „Zurück") nur, wenn direkt davor
 * oder danach ebenfalls ein Bedienrest steht. Eine Überschrift „E-Mail" vor
 * Text bleibt.
 */
export function stripWebNoise(text: string): PasteRuleResult {
  const lines = text.split("\n");
  const kinds = classifyLines(lines);
  const noise = lines.map((line, i) =>
    kinds[i] === "text" ? webNoiseKind(line) : null,
  );
  return dropLines(lines, kinds, (i) => {
    const kind = noise[i];
    if (kind === null) return false;
    if (kind !== "word") return true;
    return (i > 0 && noise[i - 1] !== null) || noise[i + 1] != null;
  });
}

// ---------------------------------------------------------------------------
// fragments, spaces (Regeln aus textCleanup, nur außerhalb geschützter Zeilen)
// ---------------------------------------------------------------------------

/** Regel auf zusammenhängende Läufe ungeschützter Zeilen anwenden. */
function mapRuns(
  text: string,
  skip: (kind: Kind) => boolean,
  rule: (run: string) => PasteRuleResult,
): PasteRuleResult {
  const lines = text.split("\n");
  const kinds = classifyLines(lines);
  const out: string[] = [];
  let changes = 0;
  let run: string[] = [];
  const flush = () => {
    if (run.length === 0) return;
    const result = rule(run.join("\n"));
    changes += result.changes;
    for (const line of result.text.split("\n")) out.push(line);
    run = [];
  };
  for (let i = 0; i < lines.length; i += 1) {
    if (skip(kinds[i])) {
      flush();
      out.push(lines[i]);
    } else run.push(lines[i]);
  }
  flush();
  return { text: out.join("\n"), changes };
}

/** Abgerissene Zeilen verbinden; Code und Blockzeilen trennen immer. */
const stripFragments = (text: string) =>
  mapRuns(text, (kind) => kind === "code" || kind === "block", joinFragments);

/** Leerzeichen bereinigen, nie in Code. */
const stripSpaces = (text: string) =>
  mapRuns(text, (kind) => kind === "code", tidySpaces);

// ---------------------------------------------------------------------------
// blankLines
// ---------------------------------------------------------------------------

/**
 * Drei und mehr Leerzeilen in Folge zu einer, Leerzeilen am Anfang und Ende
 * weg. Ein einzelner Zeilenumbruch am Ende zählt nicht als Änderung. In Code
 * bleiben Leerzeilen, wie sie sind.
 */
export function collapseBlankLines(text: string): PasteRuleResult {
  const source = text.split("\n");
  const kinds = classifyLines(source);
  const lines = source.map((l, i) => (kinds[i] === "blank" ? "" : l));
  let changes = 0;
  let start = 0;
  while (start < lines.length && kinds[start] === "blank") start += 1;
  let end = lines.length;
  while (end > start && kinds[end - 1] === "blank") end -= 1;
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
    if (kinds[i] === "blank") {
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
  repeated: stripRepeated,
  control: stripControl,
  pageNumbers: stripPageNumbers,
  webNoise: stripWebNoise,
  fragments: stripFragments,
  spaces: stripSpaces,
  blankLines: collapseBlankLines,
};

/** Kurzer Einzeiler: nichts zu bereinigen, nichts anfassen. */
const isShortLine = (text: string) => {
  const t = text.trim();
  return !/[\r\n\f]/.test(t) && t.length < SHORT_CHARS;
};

/**
 * Eingefügten Text nach festen Regeln bereinigen. Die Regeln laufen so oft,
 * bis sich nichts mehr ändert (Fixpunkt, höchstens {@link MAX_PASSES}
 * Durchläufe als Schutz): was eine Regel freilegt (Bedienrest nach dem
 * Verbinden, Lücke nach einer entfernten Seitenzahl), erledigt der nächste
 * Durchlauf. Ein zweiter Aufruf auf dem Ergebnis ändert deshalb nichts mehr.
 * Kopf- und Fußzeilen werden in jedem Durchlauf gesucht, brauchen aber
 * Seitengrenzen; nach dem ersten Durchlauf sind Seitenzahlzeilen und
 * Formfeeds weg, spätere Durchläufe finden dann keine mehr.
 */
export function cleanPasted(
  text: string,
  opts: PasteCleanOptions = {},
): PasteCleanResult {
  if (isShortLine(text)) return { text, changes: 0, items: [] };
  const wanted = opts.rules ?? PASTE_RULES;
  const totals = new Map<PasteRule, number>();
  let current = text.replace(/\r\n?/g, "\n");
  for (let pass = 0; pass < MAX_PASSES; pass += 1) {
    let passChanges = 0;
    for (const rule of PASTE_RULES) {
      if (!wanted.includes(rule)) continue;
      const result = RULE_FNS[rule](current);
      current = result.text;
      if (result.changes === 0) continue;
      passChanges += result.changes;
      totals.set(rule, (totals.get(rule) ?? 0) + result.changes);
    }
    if (passChanges === 0) break;
  }
  const items: PasteItem[] = [];
  for (const rule of PASTE_RULES) {
    const count = totals.get(rule);
    if (count) items.push({ rule, count });
  }
  const changes = items.reduce((sum, item) => sum + item.count, 0);
  return { text: current, changes, items };
}
