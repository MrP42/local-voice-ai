/**
 * ISBN im Text finden und Buchdaten in die Titelei schreiben (lva#149).
 * Die Abfrage selbst macht `isbn_lookup` im Rust-Kern; hier nur Text.
 */

/** Titelei, wie sie der EPUB-Import voranstellt: `# Titel`, Leerzeile, `von …`. */
const TITLE_PAGE = /^(\s*)# [^\n]*\n[ \t]*\n(von \p{Lu}[^\n.!?…]{0,200}\n)/u;

/**
 * Erste ISBN im Text (meist im Impressum), als Vorschlag für den Dialog.
 * Geprüft wird nur die Stellenzahl; die Prüfziffer prüft der Kern.
 */
export function findIsbn(text: string): string {
  const re =
    /ISBN(?:-1[03])?:?[ \t\u00a0]*((?:97[89][- ]?)?(?:\d[- ]?){9}[\dXx])/g;
  for (const m of text.matchAll(re)) {
    const digits = m[1].replace(/[- ]/g, "");
    if (digits.length === 10 || digits.length === 13) return m[1];
  }
  return "";
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
  const byLine = authors ? `von ${authors}\n` : (m?.[2] ?? "");
  const head = `# ${title}\n\n${byLine}`;
  if (m) return text.slice(0, m[1].length) + head + text.slice(m[0].length);
  const rest = text.replace(/^\s+/, "");
  return rest ? `${head}${byLine ? "\n" : ""}${rest}` : head;
}
