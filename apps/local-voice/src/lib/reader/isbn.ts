/**
 * ISBN im Text finden und Buchdaten in die Titelei schreiben (lva#149).
 * Die Abfrage selbst macht `isbn_lookup` im Rust-Kern; hier nur Text.
 */

/**
 * Titelei, wie sie der EPUB-Import voranstellt: `# Titel`, Leerzeile, `von …`.
 * Die Autorzeile darf Initialen haben („von A. B. Müller“), endet aber nicht
 * wie ein Satz — „von Neuem begann der Tag.“ ist Text.
 */
const TITLE_PAGE =
  /^(\s*)# [^\r\n]*\r?\n[ \t]*\r?\n(von \p{Lu}(?:[^\r\n]{0,200}[^\r\n.!?…:;,])?[ \t]*(?:\r?\n|$))/u;

/** Prüfziffer wie im Kern (`normalize_isbn`): ISBN-10 oder ISBN-13 (978/979). */
export function isValidIsbn(raw: string): boolean {
  const d = raw.replace(/[- ]/g, "").toUpperCase();
  if (/^\d{9}[\dX]$/.test(d)) {
    let sum = 0;
    for (let i = 0; i < 10; i++)
      sum += (d[i] === "X" ? 10 : Number(d[i])) * (10 - i);
    return sum % 11 === 0;
  }
  if (/^97[89]\d{10}$/.test(d)) {
    let sum = 0;
    for (let i = 0; i < 13; i++) sum += Number(d[i]) * (i % 2 ? 3 : 1);
    return sum % 10 === 0;
  }
  return false;
}

/**
 * ISBN im Text (meist im Impressum), als Vorschlag für den Dialog: die erste
 * mit passender Prüfziffer, sonst die erste der Form nach.
 */
export function findIsbn(text: string): string {
  const re =
    /ISBN(?:-1[03])?:?[ \t\u00a0]*((?:97[89][- ]?)?(?:\d[- ]?){9}[\dXx])/gi;
  let fallback = "";
  for (const m of text.matchAll(re)) {
    if (isValidIsbn(m[1])) return m[1];
    const digits = m[1].replace(/[- ]/g, "");
    if (!fallback && (digits.length === 10 || /^\d{13}$/.test(digits)))
      fallback = m[1];
  }
  return fallback;
}

const oneLine = (s: string) =>
  s
    .replace(/\s+/g, " ")
    .replace(/^#+\s*/, "")
    .trim();

/**
 * Titel und Autoren an den Textanfang: eine vorhandene Titelei (Überschrift
 * mit Autorzeile) wird ersetzt, sonst eine neue vorangestellt — eine bloße
 * Überschrift kann ein Kapitel sein und bleibt stehen. Ohne Autoren bleibt
 * eine vorhandene Autorzeile stehen.
 */
export function applyBookTitle(
  text: string,
  book: { title: string; authors: string[] },
): string {
  const title = oneLine(book.title);
  if (!title) return text;
  const authors = book.authors.map(oneLine).filter(Boolean).join(", ");
  const m = TITLE_PAGE.exec(text);
  const kept = m?.[2]?.replace(/[ \t]*\r?\n?$/, "\n") ?? "";
  const byLine = authors ? `von ${authors}\n` : kept;
  const head = `# ${title}\n\n${byLine}`;
  if (m) return text.slice(0, m[1].length) + head + text.slice(m[0].length);
  const rest = text.replace(/^\s+/, "");
  return rest ? `${head}${byLine ? "\n" : ""}${rest}` : head;
}
