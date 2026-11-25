// Datums- und Zeittexte der Terminfindung, immer deutsche 24-Stunden-Zeit (wie die Planung).

const fmt = (o: Intl.DateTimeFormatOptions) =>
  new Intl.DateTimeFormat("de-DE", { hourCycle: "h23", ...o });

const WT = fmt({ weekday: "short" });
const DM = fmt({ day: "2-digit", month: "2-digit" });
const HM = fmt({ hour: "2-digit", minute: "2-digit" });
const LANG = fmt({
  weekday: "long",
  day: "2-digit",
  month: "2-digit",
  year: "numeric",
});

export const wochentag = (ms: number) => WT.format(ms).replace(".", "");
export const tagMonat = (ms: number) => DM.format(ms);
export const uhrzeit = (ms: number) => HM.format(ms);
export const spanne = (a: number, b: number) => `${uhrzeit(a)}–${uhrzeit(b)}`;
export const vorschlagKurz = (a: number, b: number) =>
  `${wochentag(a)} ${tagMonat(a)} · ${spanne(a, b)}`;
export const vorschlagLang = (a: number, b: number) =>
  `${LANG.format(a)} · ${spanne(a, b)}`;

/** Kalenderwoche nach ISO 8601. */
export function kalenderwoche(ms: number): number {
  const d = new Date(ms);
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  const wt = t.getUTCDay() || 7;
  t.setUTCDate(t.getUTCDate() + 4 - wt);
  const j = new Date(Date.UTC(t.getUTCFullYear(), 0, 1));
  return Math.ceil(((t.getTime() - j.getTime()) / 86_400_000 + 1) / 7);
}

/** Wert für `<input type="datetime-local">` und zurück. */
export function eingabeAusMs(ms: number): string {
  const d = new Date(ms);
  const z = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())}T${z(d.getHours())}:${z(d.getMinutes())}`;
}
export function msAusEingabe(s: string): number | null {
  const ms = new Date(s).getTime();
  return Number.isFinite(ms) ? ms : null;
}
