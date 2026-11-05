/**
 * Seitenzahl an einer Stelle des Buchtexts (lva#309): die letzte Seitenmarke
 * `{{S. n}}` vor dem Offset (UTF-16, wie Reader, Lesezeichen und Belegsprung).
 * Ohne Marke vor der Stelle `null`; der Bereichsknopf sagt dann „Bis hier“.
 */
const MARK = /\{\{S\. ?(\d{1,5})\}\}/g;

export function pageAt(text: string, offset: number): number | null {
  if (offset <= 0) return null;
  let found: number | null = null;
  MARK.lastIndex = 0;
  for (let m = MARK.exec(text); m; m = MARK.exec(text)) {
    if (m.index >= offset) break;
    found = Number(m[1]);
  }
  return found;
}

/**
 * Konkrete Grenze für den Bereichsknopf: Seite aus der letzten Seitenmarke,
 * sonst Kapitel (Index + 1), bei Grenze 0 der Anfang. `here` nur, wenn es weder
 * Seitenmarken noch ein Inhaltsverzeichnis gibt.
 */
export type ScopeLimit =
  | { kind: "page"; page: number }
  | { kind: "chapter"; chapter: number }
  | { kind: "start" }
  | { kind: "here" };

/** `chapterStarts`: Anfangsoffsets (UTF-16, aufsteigend) der Kapitel des Inhaltsverzeichnisses. */
export function scopeLimit(
  text: string,
  boundary: number | undefined,
  chapterStarts: readonly number[],
): ScopeLimit {
  if (boundary === undefined || boundary <= 0) return { kind: "start" };
  const page = pageAt(text, boundary);
  if (page !== null) return { kind: "page", page };
  let index = -1;
  for (let i = 0; i < chapterStarts.length && chapterStarts[i] <= boundary; i++)
    index = i;
  return index >= 0 ? { kind: "chapter", chapter: index + 1 } : { kind: "here" };
}

/** Schlüssel unter `tts.bookTalk.scope.` und Platzhalter für Knopf, Ansage, `aria-label`. */
export function scopeText(
  limit: ScopeLimit,
  variant: "label" | "aria" | "phrase",
): { key: string; values: { page?: number; chapter?: number } } {
  const name =
    limit.kind === "page"
      ? "Page"
      : limit.kind === "chapter"
        ? "Chapter"
        : limit.kind === "start"
          ? "Start"
          : "Here";
  const prefix = variant === "label" ? "label" : variant === "aria" ? "aria" : "limit";
  return {
    key: `${prefix}${name}`,
    values:
      limit.kind === "page"
        ? { page: limit.page }
        : limit.kind === "chapter"
          ? { chapter: limit.chapter }
          : {},
  };
}
