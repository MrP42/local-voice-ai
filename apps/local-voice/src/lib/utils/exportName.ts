/**
 * Dateiname eines Vorlese-Exports: Titel des Arbeitsblatts und Zeitpunkt.
 *
 * Bis dahin hiess jeder Export "vorlesen" und ueberschrieb den vorherigen —
 * wer zwei Fassungen eines Hoerspiels erzeugte, hatte am Ende eine. Der
 * Zeitstempel macht sie unterscheidbar, der Titel wiedererkennbar.
 *
 * @param title Titel des Arbeitsblatts; leer oder nur Leerzeichen zaehlt als
 *   kein Titel.
 * @param fallback Wie die Datei heisst, wenn es keinen Titel gibt.
 * @param ext Endung ohne Punkt.
 * @param now Zeitpunkt — als Parameter, damit der Name pruefbar ist.
 */
export function exportFileName(
  title: string | undefined | null,
  fallback: string,
  ext: string,
  now: Date = new Date(),
): string {
  // Windows verbietet \ / : * ? " < > | im Dateinamen; Leerzeichen werden zu
  // Bindestrichen, damit der Name auch in einer Konsole handlich bleibt.
  // Punkte am Ende entfernt Windows stillschweigend — also gleich hier.
  const safe = (title?.trim() || fallback)
    .replace(/[\\/:*?"<>|]/g, "")
    .replace(/\s+/g, "-")
    .slice(0, 60)
    .replace(/^[-.]+|[-.]+$/g, "");
  const pad = (value: number) => String(value).padStart(2, "0");
  const stamp =
    `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}` +
    `_${pad(now.getHours())}${pad(now.getMinutes())}`;
  return `${safe || fallback}_${stamp}.${ext}`;
}
