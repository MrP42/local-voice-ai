import i18n from "@/i18n";

// Datums- und Zeittexte der Terminfindung in der App-Sprache, immer 24-Stunden-Zeit (wie die Planung).

type Art = "wt" | "dm" | "hm" | "lang";
const OPTIONEN: Record<Art, Intl.DateTimeFormatOptions> = {
  wt: { weekday: "short" },
  dm: { day: "2-digit", month: "2-digit" },
  hm: { hour: "2-digit", minute: "2-digit" },
  lang: { weekday: "long", day: "2-digit", month: "2-digit", year: "numeric" },
};
const cache = new Map<string, Intl.DateTimeFormat>();
function fmt(art: Art): Intl.DateTimeFormat {
  const sprache = i18n.language || "de";
  const k = `${sprache}|${art}`;
  let f = cache.get(k);
  if (!f) {
    try {
      f = new Intl.DateTimeFormat(sprache, {
        hourCycle: "h23",
        ...OPTIONEN[art],
      });
    } catch {
      f = new Intl.DateTimeFormat("de-DE", {
        hourCycle: "h23",
        ...OPTIONEN[art],
      });
    }
    cache.set(k, f);
  }
  return f;
}

export const wochentag = (ms: number) => fmt("wt").format(ms).replace(".", "");
export const tagMonat = (ms: number) => fmt("dm").format(ms);
export const uhrzeit = (ms: number) => fmt("hm").format(ms);
export const spanne = (a: number, b: number) => `${uhrzeit(a)}–${uhrzeit(b)}`;
export const vorschlagKurz = (a: number, b: number) =>
  `${wochentag(a)} ${tagMonat(a)} · ${spanne(a, b)}`;
export const vorschlagLang = (a: number, b: number) =>
  `${fmt("lang").format(a)} · ${spanne(a, b)}`;

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
