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

/** Eigene Adresse: nur https, Platzhalter `{q}` erst nach dem Host (Pfad
 *  oder Suchteil), damit der Begriff nie die Zieladresse bestimmt. */
export function validSearchUrl(url: string): boolean {
  const u = url.trim();
  if (!u.startsWith("https://") || !u.includes("{q}")) return false;
  const rest = u.slice("https://".length);
  const hostEnd = rest.search(/[/?#]/);
  if (hostEnd < 0 || rest.slice(0, hostEnd).includes("{q}")) return false;
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

/** Modelltext als reiner Text: Klammern, Sprechermarken, Seitenmarken und
 *  Markdown-Zeichen wären sonst Steuerung für die Vorlese-Engine. */
const plain = (s: string) =>
  s
    .replace(/[[\]{}<>*_#`|~]/g, " ")
    .replace(/\s+/g, " ")
    .trim();

/** Text, der beim Öffnen des Fensters vorgelesen wird. Modellwissen ist
 *  ausdrücklich angesagt, nie als Aussage des Buchs. */
export function spokenExplanation(
  term: { term: string; from_book: string; general: string },
  labels: { fromBook: string; general: string; none: string },
): string {
  const parts = [`${plain(term.term)}.`];
  if (plain(term.from_book))
    parts.push(`${labels.fromBook}: ${plain(term.from_book)}`);
  if (plain(term.general))
    parts.push(`${labels.general}: ${plain(term.general)}`);
  if (parts.length === 1) parts.push(labels.none);
  return parts.join("\n\n");
}
