/**
 * Der Satzzaehler der Vorlesen-Oberflaechen (Vorlesen-Seite und Wiedergabeleiste der
 * Aufnahmen): "n/m" = der Satz, der gerade zu hoeren ist (1-basiert), und die Gesamtzahl.
 *
 * Das Backend meldet zwei Ereignisse:
 * - `tts-speak-progress {position, total}`: `position` = vollstaendig gesprochene Saetze.
 *   Gleich zu Beginn eines Laufs (Start, Weiter, Satzsprung, Neustart) kommt eines mit der
 *   Startposition -- der Zaehler steht damit sofort, nicht erst nach dem ersten Satz.
 * - `tts-current-sentence {index, text}`: der Satz, der jetzt zu spielen beginnt.
 */

/**
 * Welchen "aktuellen Satz" man nach einer Fortschrittsmeldung noch kennt: nur den, der zur
 * gemeldeten Position passt. Nach einem Sprung (oder wenn ein Satz fertig ist) gilt der
 * Satz davor nicht mehr -- bis `tts-current-sentence` den neuen meldet, zaehlt die Position.
 */
export const sentenceAfterProgress = <T extends { index: number }>(
  sentence: T | null,
  position: number,
): T | null =>
  sentence !== null && sentence.index === position ? sentence : null;

/** Der angezeigte Satz (1-basiert, hoechstens `total`), oder null, solange unbekannt. */
export const shownSentenceNumber = (
  index: number | null,
  total: number,
): number | null =>
  index !== null && total > 0 ? Math.min(index + 1, total) : null;

/**
 * Feste Breite des Zaehlers in `ch`: so viele Zeichen wie der breiteste Text hat (alle
 * Stellen belegt). Mit `tabular-nums` sind alle Ziffern gleich breit, die Schaltflaeche
 * daneben springt nicht, wenn aus "9/12" "10/12" wird.
 */
export const counterWidthCh = (
  total: number,
  format: (position: string, total: string) => string,
): number => {
  const nines = "9".repeat(String(Math.max(total, 1)).length);
  return [...format(nines, nines)].length;
};
