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
