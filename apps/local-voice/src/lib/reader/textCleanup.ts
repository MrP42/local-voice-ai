/**
 * Textbereinigung ohne KI (lva: Editor-Menü).
 *
 * Alles, was sich mit festen Regeln sicher erkennen lässt, läuft hier und
 * kostet kein einziges Token: Zeitstempel aus Transkripten, abgerissene
 * Zeilen (`J.` / `L.` / `Collins …`), Silbentrennung am Zeilenende,
 * doppelte Leerzeichen und Zeilen, die zu Absätzen gehören. Erst was danach
 * übrig bleibt (Grammatik, Gliederung), geht an ein Sprachmodell.
 *
 * Sprecher `<Name>`, Tags `[…]`, Seitenmarken `{{S. n}}` und das
 * Markdown-Subset der Buchansicht (`readerDoc.ts`) bleiben unangetastet.
 */

export interface CleanupResult {
  text: string;
  /** Anzahl der Stellen, die die Regel geändert hat. */
  changes: number;
}

export type CleanupRule = "timestamps" | "fragments" | "spaces" | "paragraphs";

/** Reihenfolge von „Alle Regeln": erst Zeitstempel weg, dann kleben die
 *  abgerissenen Zeilen sauber zusammen, zuletzt die Absätze. */
export const CLEANUP_RULES: readonly CleanupRule[] = [
  "timestamps",
  "fragments",
  "spaces",
  "paragraphs",
];

/** Zeitstempel am Zeilenanfang: `[0:00]:`, `[1:02:03] `, `(00:12)`, `[00:01.5]`. */
const TIMESTAMP =
  /^[ \t]*[[(]\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,3})?[\])][ \t]*(?::[ \t]*)?/;
/** Rest eines ausgeblendeten Zeitstempels: Zeile beginnt mit `: `. */
const LONE_COLON = /^[ \t]*:[ \t]+(?=\S)/;
/** Zeilen, die die Buchansicht als eigenen Block liest. */
const BLOCK_START =
  /^\s*(?:#{1,3}\s|>\s?|[-*]\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$|⁂)/;
/** Sprecher am Zeilenanfang `<Name>` / `<Name:Stil>`. */
const SPEAKER_START = /^\s*<[^<>\n]{1,40}>/;
/** Satzende (auch mit schließendem Anführungszeichen oder Klammer). */
const SENTENCE_END = /[.!?…:;]["'»«“”‘’)\]]*\s*$/;
/** Nur Initialen oder ein abgerissener Einzelbuchstabe: `J.`, `J. L.`, `L`. */
const INITIALS = /^(?:(?:\p{Lu}\.\s*){1,3}|\p{Lu})$/u;
/** Silbentrennung am Zeilenende: Kleinbuchstabe + Bindestrich. */
const HYPHEN_END = /\p{Ll}-$/u;
const STARTS_LOWER = /^\p{Ll}/u;

const isPlain = (line: string) =>
  line.trim().length > 0 &&
  !BLOCK_START.test(line) &&
  !SPEAKER_START.test(line) &&
  !/^\s*\{\{S\./.test(line);

/** Zeitstempel am Zeilenanfang (und den übrig gebliebenen `: `) entfernen. */
export function stripTimestamps(text: string): CleanupResult {
  let changes = 0;
  const out = text.split("\n").map((line) => {
    const next = line.replace(TIMESTAMP, "").replace(LONE_COLON, "");
    if (next !== line) changes += 1;
    return next;
  });
  return { text: out.join("\n"), changes };
}

/**
 * Abgerissene Zeilen zusammenfügen:
 * - Initialen allein auf einer Zeile (`J.` ↵ `L.` ↵ `Collins hat …`),
 * - Silbentrennung am Zeilenende (`Ge-` ↵ `danken`),
 * - Zeile ohne Satzende, die nächste beginnt klein (`… auf www.x.com` ↵
 *   `zu schreiben begann.`).
 * Leerzeilen, Sprecherwechsel und Markdown-Blöcke trennen immer.
 */
export function joinFragments(text: string): CleanupResult {
  const lines = text.split("\n");
  const out: string[] = [];
  let changes = 0;
  for (const raw of lines) {
    const prev = out.length > 0 ? out[out.length - 1] : null;
    if (prev !== null && isPlain(prev) && isPlain(raw)) {
      // Ein Zeitstempel vor dem Bruchstück gehört zum Satz davor.
      const body = raw.replace(TIMESTAMP, "").replace(LONE_COLON, "").trim();
      const prevBody = prev
        .replace(TIMESTAMP, "")
        .replace(LONE_COLON, "")
        .trim();
      if (body.length > 0) {
        if (HYPHEN_END.test(prev.trimEnd()) && STARTS_LOWER.test(body)) {
          out[out.length - 1] = prev.trimEnd().slice(0, -1) + body;
          changes += 1;
          continue;
        }
        if (
          INITIALS.test(prevBody) ||
          (!SENTENCE_END.test(prev) && STARTS_LOWER.test(body))
        ) {
          out[out.length - 1] = `${prev.trimEnd()} ${body}`;
          changes += 1;
          continue;
        }
      }
    }
    out.push(raw);
  }
  return { text: out.join("\n"), changes };
}

/** Doppelte Leerzeichen, Leerzeichen vor Satzzeichen und am Zeilenende. */
export function tidySpaces(text: string): CleanupResult {
  let changes = 0;
  const out = text.split("\n").map((line) => {
    const lead = /^\s*/.exec(line)?.[0] ?? "";
    const body = line
      .slice(lead.length)
      .replace(/[ \t ]{2,}/g, " ")
      .replace(/(\p{L}|\d) +([,.;!?])(?=\s|$)/gu, "$1$2")
      .replace(/[ \t]+$/, "");
    // Einrückung nur bei Fließtext entfernen; Listen/Zitate behalten sie.
    const next = (isPlain(line) ? "" : lead) + body;
    if (next !== line) changes += 1;
    return next;
  });
  return { text: out.join("\n"), changes };
}

/** Höchstlänge eines zusammengefügten Absatzes in Zeichen. */
const PARAGRAPH_CHARS = 700;

/**
 * Einzeilige Sätze (typisch für Transkripte) zu Absätzen verbinden: Zeilen
 * werden mit Leerzeichen gereiht, bis der Absatz etwa {@link PARAGRAPH_CHARS}
 * Zeichen hat und ein Satz endet. Absätze trennt eine Leerzeile. Zeilen mit
 * Zeitstempel, Sprecher oder Markdown bleiben eigene Zeilen.
 */
export function linesToParagraphs(text: string): CleanupResult {
  const lines = text.split("\n");
  const out: string[] = [];
  let changes = 0;
  let para: string | null = null;
  const flush = () => {
    if (para === null) return;
    out.push(para);
    para = null;
  };
  const blankBefore = () => {
    if (out.length > 0 && out[out.length - 1].trim() !== "") out.push("");
  };
  for (const line of lines) {
    if (line.trim() === "") {
      flush();
      if (out.length > 0 && out[out.length - 1].trim() !== "") out.push("");
      continue;
    }
    const joinable = isPlain(line) && !TIMESTAMP.test(line);
    if (!joinable) {
      flush();
      out.push(line);
      continue;
    }
    if (para === null) {
      para = line.trim();
      continue;
    }
    if (para.length >= PARAGRAPH_CHARS && SENTENCE_END.test(para)) {
      flush();
      blankBefore();
      para = line.trim();
      changes += 1;
      continue;
    }
    para = `${para} ${line.trim()}`;
    changes += 1;
  }
  flush();
  const joined = out.join("\n");
  return { text: joined, changes: joined === text ? 0 : changes };
}

const RULE_FNS: Record<CleanupRule, (text: string) => CleanupResult> = {
  timestamps: stripTimestamps,
  fragments: joinFragments,
  spaces: tidySpaces,
  paragraphs: linesToParagraphs,
};

/** Eine oder mehrere Regeln in fester Reihenfolge anwenden. */
export function applyCleanup(
  text: string,
  rules: readonly CleanupRule[],
): CleanupResult {
  let current = text;
  let changes = 0;
  for (const rule of CLEANUP_RULES) {
    if (!rules.includes(rule)) continue;
    const result = RULE_FNS[rule](current);
    current = result.text;
    changes += result.changes;
  }
  return { text: current, changes };
}

/** Wie {@link applyCleanup}, aber nur auf die Zeilen, die `[start, end)`
 *  berühren (Markierung). Ohne Markierung gilt der ganze Text. */
export function applyCleanupInRange(
  text: string,
  rules: readonly CleanupRule[],
  start: number,
  end: number,
): CleanupResult {
  if (start >= end) return applyCleanup(text, rules);
  const from = text.lastIndexOf("\n", Math.max(0, start - 1)) + 1;
  const nl = text.indexOf("\n", end - 1);
  const to = nl < 0 ? text.length : nl;
  const result = applyCleanup(text.slice(from, to), rules);
  return {
    text: text.slice(0, from) + result.text + text.slice(to),
    changes: result.changes,
  };
}

/** Zeitstempel / Sprecher / Markdown am Zeilenanfang: bleibt bei KI-Korrektur
 *  erhalten und geht nicht ans Modell (spart Token, nichts geht verloren). */
const LINE_PREFIX =
  /^(?:[ \t]*[[(]\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,3})?[\])][ \t]*(?::[ \t]*)?|[ \t]*<[^<>\n]{1,40}>[ \t]*|[ \t]*#{1,3}[ \t]+|[ \t]*>[ \t]?|[ \t]*[-*][ \t]+|[ \t]*:[ \t]+)*/;

export function splitLinePrefix(line: string): {
  prefix: string;
  body: string;
} {
  const prefix = LINE_PREFIX.exec(line)?.[0] ?? "";
  return { prefix, body: line.slice(prefix.length) };
}
