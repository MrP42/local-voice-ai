/**
 * Reine Hilfen des Editor-Menüs: Suchen/Ersetzen und Formatieren im
 * Markdown-Subset der Buchansicht (`readerDoc.ts`). Alle Offsets sind
 * UTF-16-Offsets im Seitentext, wie im Editor.
 */

export interface FindOptions {
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
}

export interface TextRange {
  start: number;
  end: number;
}

const escapeRegex = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Suchmuster bauen; `null` bei leerer Suche oder ungültigem Ausdruck. */
export function findPattern(query: string, opts: FindOptions): RegExp | null {
  if (!query) return null;
  let source = opts.regex ? query : escapeRegex(query);
  if (opts.wholeWord)
    source = `(?<![\\p{L}\\p{N}_])(?:${source})(?![\\p{L}\\p{N}_])`;
  try {
    return new RegExp(source, `gu${opts.caseSensitive ? "" : "i"}`);
  } catch {
    return null;
  }
}

/** Alle Fundstellen (leere Treffer eines Ausdrucks werden übersprungen). */
export function findAll(
  text: string,
  query: string,
  opts: FindOptions,
  limit = 5000,
): TextRange[] {
  const re = findPattern(query, opts);
  if (!re) return [];
  const out: TextRange[] = [];
  for (const m of text.matchAll(re)) {
    if (m[0].length === 0) continue;
    out.push({ start: m.index ?? 0, end: (m.index ?? 0) + m[0].length });
    if (out.length >= limit) break;
  }
  return out;
}

/**
 * Ersatztext wie bei `String.replace` auflösen: `$$`, `$&`, `$1`–`$99`
 * (zweistellig nur, wenn es so viele Gruppen gibt) und `$<name>`.
 */
function expand(
  replacement: string,
  match: string,
  groups: (string | undefined)[],
  named: Record<string, string | undefined> | undefined,
): string {
  return replacement.replace(
    /\$(\$|&|<([^>]*)>|(\d)(\d)?)/g,
    (whole, tok: string, name?: string, d1?: string, d2?: string) => {
      if (tok === "$") return "$";
      if (tok === "&") return match;
      if (name !== undefined) return named ? (named[name] ?? "") : whole;
      const two = d2 !== undefined ? Number(d1! + d2) : 0;
      if (two >= 1 && two <= groups.length) return groups[two - 1] ?? "";
      const one = Number(d1);
      if (one >= 1 && one <= groups.length)
        return (groups[one - 1] ?? "") + (d2 ?? "");
      return whole;
    },
  );
}

/** Alle Treffer samt Ersatztext, im Kontext des ganzen Texts (Lookbehind,
 *  `^`, `\b` wirken wie in der Suche). Leere Treffer zählen nicht. */
function replacements(
  text: string,
  query: string,
  replacement: string,
  opts: FindOptions,
): { start: number; end: number; insert: string }[] {
  const re = findPattern(query, opts);
  if (!re) return [];
  const out: { start: number; end: number; insert: string }[] = [];
  for (const m of text.matchAll(re)) {
    if (m[0].length === 0) continue;
    const start = m.index ?? 0;
    out.push({
      start,
      end: start + m[0].length,
      insert: opts.regex
        ? expand(replacement, m[0], m.slice(1), m.groups)
        : replacement,
    });
  }
  return out;
}

/** Ersatztext für den Treffer, der bei `start` beginnt. */
export function replacementAt(
  text: string,
  start: number,
  query: string,
  replacement: string,
  opts: FindOptions,
): string | null {
  return (
    replacements(text, query, replacement, opts).find((r) => r.start === start)
      ?.insert ?? null
  );
}

export function replaceAll(
  text: string,
  query: string,
  replacement: string,
  opts: FindOptions,
): { text: string; changes: number } {
  const hits = replacements(text, query, replacement, opts);
  if (hits.length === 0) return { text, changes: 0 };
  let next = "";
  let at = 0;
  for (const h of hits) {
    next += text.slice(at, h.start) + h.insert;
    at = h.end;
  }
  next += text.slice(at);
  return { text: next, changes: hits.length };
}

export type FormatKind =
  | "h1"
  | "h2"
  | "h3"
  | "paragraph"
  | "bold"
  | "italic"
  | "quote"
  | "list"
  | "rule";

export interface FormatEdit {
  /** Zu ersetzender Bereich im alten Text … */
  start: number;
  end: number;
  /** … und der neue Inhalt dafür. */
  insert: string;
  /** Markierung danach (im neuen Text). */
  selStart: number;
  selEnd: number;
}

const BLOCK_PREFIX = /^(\s*)(#{1,3}\s+|>\s?|[-*]\s+)?/;

function lineBounds(text: string, start: number, end: number) {
  const from = text.lastIndexOf("\n", Math.max(0, start - 1)) + 1;
  const probe = end > start ? end - 1 : end;
  const nl = text.indexOf("\n", Math.max(from, probe));
  return { from, to: nl < 0 ? text.length : nl };
}

/**
 * Formatierung an Markierung bzw. Cursor. Zeilenformate (Überschrift,
 * Zitat, Liste) gelten für alle berührten Zeilen und schalten um: ist das
 * Format schon gesetzt, wird es entfernt. Fett/Kursiv umschließen die
 * Markierung (ohne Markierung: Marker setzen, Cursor dazwischen).
 */
export function applyFormat(
  text: string,
  start: number,
  end: number,
  kind: FormatKind,
): FormatEdit {
  if (kind === "bold" || kind === "italic") {
    const mark = kind === "bold" ? "**" : "*";
    const inner = text.slice(start, end);
    const n = mark.length;
    const wrapped =
      text.slice(start - n, start) === mark &&
      text.slice(end, end + n) === mark &&
      // `**fett**` ist kein kursives `*…*`.
      (kind === "bold" || text.slice(start - 2, start) !== "**");
    if (wrapped) {
      return {
        start: start - n,
        end: end + n,
        insert: inner,
        selStart: start - n,
        selEnd: end - n,
      };
    }
    return {
      start,
      end,
      insert: `${mark}${inner}${mark}`,
      selStart: start + n,
      selEnd: end + n,
    };
  }
  if (kind === "rule") {
    const { to } = lineBounds(text, end, end);
    const insert = "\n\n---\n";
    return {
      start: to,
      end: to,
      insert,
      selStart: to + insert.length,
      selEnd: to + insert.length,
    };
  }
  const { from, to } = lineBounds(text, start, end);
  const lines = text.slice(from, to).split("\n");
  const target =
    kind === "h1"
      ? "# "
      : kind === "h2"
        ? "## "
        : kind === "h3"
          ? "### "
          : kind === "quote"
            ? "> "
            : kind === "list"
              ? "- "
              : "";
  const current = (line: string) =>
    (BLOCK_PREFIX.exec(line)?.[2] ?? "").replace(/\s+$/, " ");
  const content = lines.filter((l) => l.trim() !== "");
  // Umschalten: tragen alle Zeilen schon dieses Format, wird es entfernt.
  const allSet =
    target !== "" &&
    content.length > 0 &&
    content.every((l) => current(l) === target);
  const next = lines
    .map((line) => {
      if (line.trim() === "") return line;
      const m = BLOCK_PREFIX.exec(line);
      const rest = line.slice(m?.[0].length ?? 0);
      return (allSet ? "" : target) + rest;
    })
    .join("\n");
  return {
    start: from,
    end: to,
    insert: next,
    selStart: from,
    selEnd: from + next.length,
  };
}
