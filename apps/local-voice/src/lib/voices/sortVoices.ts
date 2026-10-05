/// Stimmenlisten alphabetisch nach dem sichtbaren Namen ordnen.
///
/// Das Backend sortiert nach der technischen Kennung und byteweise — dort
/// steht "Erzähler" (Anzeigename) vor "david", und "spongebob-schwammkopf"
/// mit Anzeigename "Spongebob" sortiert nach der Kennung statt nach dem, was
/// man liest. Deshalb ordnet die Oberfläche selbst: deutsch, ohne Rücksicht
/// auf Groß-/Kleinschreibung, Zahlen natürlich ("Erzählerin 2" vor "… 10").
const collator = new Intl.Collator("de", {
  sensitivity: "base",
  numeric: true,
});

export const compareVoiceLabels = (a: string, b: string): number =>
  collator.compare(a, b);

/// Sortierte Kopie; `label` liefert den Namen, den die Liste anzeigt.
export const sortVoicesByLabel = <T>(
  items: readonly T[],
  label: (item: T) => string,
): T[] => [...items].sort((a, b) => compareVoiceLabels(label(a), label(b)));
