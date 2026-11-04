/**
 * Protokoll von Hand bearbeiten: Hilfen ohne React, damit sie sich ohne
 * Oberfläche prüfen lassen.
 */

/** Wörter eines Textes (Buchstaben und Ziffern), für den Abgleich mit dem Markdown. */
const words = (text: string): string[] => text.match(/[\p{L}\p{N}]+/gu) ?? [];

const escapeRegex = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Wo im Markdown steht der Block, den man in der Leseansicht angeklickt hat?
 * Die Leseansicht zeigt den Text ohne Auszeichnung (`**`, `- `, `[F7]`-Marken),
 * deshalb gilt zwischen zwei Wörtern alles, was kein Buchstabe und keine
 * Ziffer ist, als Trenner. Gesucht wird mit den ersten Wörtern; zuerst mit
 * acht, dann mit weniger, bis ein Treffer da ist.
 *
 * Liefert den Bereich des Treffers (Anfang des ersten bis Ende des letzten
 * gesuchten Wortes) oder `null`, wenn sich der Block nicht finden lässt
 * (dann öffnet der Editor am Anfang).
 */
export function locateInMarkdown(
  markdown: string,
  blockText: string,
): { start: number; end: number } | null {
  const all = words(blockText);
  if (all.length === 0) return null;
  const counts = [Math.min(8, all.length), 5, 3, 1].filter(
    (n, i, list) => n <= all.length && list.indexOf(n) === i,
  );
  for (const n of counts) {
    const picked = all.slice(0, n);
    const pattern = picked.map(escapeRegex).join("[^\\p{L}\\p{N}]+");
    const re = new RegExp(
      `(?<![\\p{L}\\p{N}])${pattern}(?![\\p{L}\\p{N}])`,
      "u",
    );
    const match = re.exec(markdown);
    if (match)
      return { start: match.index, end: match.index + match[0].length };
  }
  return null;
}

/** Fehlercodes von `meetings_update_document_text`, die eine eigene Meldung haben. */
export const SAVE_ERROR_CODES = [
  "stale_document",
  "document_not_found",
  "document_not_editable",
  "document_empty",
  "document_too_large",
] as const;

export type SaveErrorCode = (typeof SAVE_ERROR_CODES)[number];

/** Code aus dem Fehlertext des Backends (`code` oder `code: detail`). */
export function saveErrorCode(raw: string): SaveErrorCode | "other" {
  const code = raw.split(":")[0].trim();
  return (SAVE_ERROR_CODES as readonly string[]).includes(code)
    ? (code as SaveErrorCode)
    : "other";
}
