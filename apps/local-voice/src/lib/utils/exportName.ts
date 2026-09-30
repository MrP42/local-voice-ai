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

/** Laenge des Stamms (Stimme + Zusatz), ohne Zeitstempel und Endung. */
const AUDIO_STEM_MAX = 24;

/** Windows-sicher: verbotene Zeichen raus, Leerraum zu `-`, Rand bereinigen. */
const safeName = (value: string | undefined | null): string =>
  (value ?? "")
    .trim()
    .replace(/[\\/:*?"<>|]/g, "")
    .replace(/\s+/g, "-")
    .replace(/^[-.]+|[-.]+$/g, "");

/**
 * Standardname einer erzeugten Audiodatei: `<Stamm>_<JJJJ-MM-TT_HHMM>.<ext>`.
 *
 * Der Seitentitel taugt dafuer nicht — er ist lang, und die Dateileiste zeigt
 * nur den Anfang. Der Stamm besteht daher aus der Stimme und optional einem
 * Zusatz (Sprache, "Zusammenfassung") und bleibt kurz, damit der Zeitstempel
 * nie aus der Spalte rutscht. Ist der Stamm zu lang, wird die Stimme gekuerzt:
 * der Zusatz unterscheidet Fassungen und bleibt deshalb ganz.
 *
 * @param voice Anzeigename der Stimme; leer → `fallback`.
 * @param suffix Zusatz wie `EN`; leer → keiner.
 * @param fallback Stamm, wenn es keine (brauchbare) Stimme gibt.
 * @param ext Endung ohne Punkt.
 * @param now Zeitpunkt — als Parameter, damit der Name pruefbar ist.
 */
export function audioExportName({
  voice,
  suffix,
  fallback,
  ext,
  now = new Date(),
}: {
  voice: string | undefined | null;
  suffix?: string | null;
  fallback: string;
  ext: string;
  now?: Date;
}): string {
  const extra = safeName(suffix);
  const extraPart = extra ? `-${extra}` : "";
  // Der Zusatz behaelt seinen Platz; die Stimme bekommt den Rest. Ein
  // Zusatz, der allein schon zu lang ist, wird als letzter Ausweg gekappt.
  const room = Math.max(1, AUDIO_STEM_MAX - extraPart.length);
  const base = safeName(voice) || safeName(fallback) || fallback;
  const head =
    base.slice(0, room).replace(/[-.]+$/g, "") || base.slice(0, room);
  const stem = `${head}${extraPart}`.slice(0, AUDIO_STEM_MAX);
  const pad = (value: number) => String(value).padStart(2, "0");
  const stamp =
    `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}` +
    `_${pad(now.getHours())}${pad(now.getMinutes())}`;
  return `${stem}_${stamp}.${ext}`;
}

/**
 * Teilt einen Dateinamen fuer die mittige Kuerzung in der Dateileiste:
 * `head` darf schrumpfen, `tail` bleibt stehen. Der Schwanz beginnt am
 * letzten `_JJJJ-MM-TT_HHMM` (der Zeitstempel ist das Unterscheidende),
 * sonst sind es die letzten 12 Zeichen inklusive Endung.
 */
export function splitFileNameTail(name: string): {
  head: string;
  tail: string;
} {
  const stamp = [...name.matchAll(/_\d{4}-\d{2}-\d{2}_\d{4}/g)].pop();
  // Nur wenn der Zeitstempel wirklich am Ende steht: ein langer fester
  // Schwanz koennte nicht schrumpfen und spraenge die Zeile.
  const cut =
    stamp?.index !== undefined && name.length - stamp.index <= 24
      ? stamp.index
      : Math.max(0, name.length - 12);
  return { head: name.slice(0, cut), tail: name.slice(cut) };
}
