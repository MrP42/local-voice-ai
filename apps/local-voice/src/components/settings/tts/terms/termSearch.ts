/**
 * „Mehr erfahren“ (lva#185): Suche im Browser. Standard ist Wikipedia in der
 * Sprache der Oberfläche; eine eigene Adresse ist möglich. Später kommt hier
 * die Schnittstelle zum Recherche-Modul an.
 */
export const SEARCH_PRESETS = {
  wikipedia: "https://{lang}.wikipedia.org/w/index.php?search={q}",
  duckduckgo: "https://duckduckgo.com/?q={q}",
} as const;
export type SearchPreset = keyof typeof SEARCH_PRESETS | "custom";

/** Eigene Adresse: nur https und mit Platzhalter `{q}`. */
export function validSearchUrl(url: string): boolean {
  const u = url.trim();
  if (!u.startsWith("https://") || !u.includes("{q}")) return false;
  try {
    new URL(u.replace(/\{q\}|\{lang\}/g, "x"));
    return true;
  } catch {
    return false;
  }
}

export function searchUrl(
  preset: SearchPreset,
  custom: string,
  term: string,
  lang: string,
): string {
  const tpl =
    preset === "custom" && validSearchUrl(custom)
      ? custom.trim()
      : SEARCH_PRESETS[preset === "custom" ? "wikipedia" : preset];
  const safeLang = /^[a-z]{2,3}$/.test(lang) ? lang : "de";
  return tpl
    .replace(/\{lang\}/g, safeLang)
    .replace(/\{q\}/g, encodeURIComponent(term.trim()));
}

/** Text, der beim Öffnen des Fensters vorgelesen wird. Modellwissen ist
 *  ausdrücklich angesagt, nie als Aussage des Buchs. */
export function spokenExplanation(
  term: { term: string; from_book: string; general: string },
  labels: { fromBook: string; general: string; none: string },
): string {
  const parts = [`${term.term}.`];
  if (term.from_book.trim())
    parts.push(`${labels.fromBook}: ${term.from_book.trim()}`);
  if (term.general.trim())
    parts.push(`${labels.general}: ${term.general.trim()}`);
  if (parts.length === 1) parts.push(labels.none);
  return parts.join("\n\n");
}
