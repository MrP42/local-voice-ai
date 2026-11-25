import type {
  Antwort,
  Availability,
  Ortszeit,
  PersonAvailability,
  Person,
  PollDefinition,
  PollHistory,
  Vorschlag,
  Wert,
} from "@/bindings";

// Terminfindung (lva#845 TF5): reine Hilfsfunktionen der Oberfläche. Die Bewertung „bester
// Zeitpunkt“ rechnet das Crate (poll_rank); hier wird nur angezeigt, nie gewertet.

export const MIN = 60_000;
export const RASTER_MIN = 30;
/** Sichtbarer Tagesausschnitt des Wochenrasters (Ortszeit). */
export const TAG_VON = 8;
export const TAG_BIS = 18;
export const ZEILEN = ((TAG_BIS - TAG_VON) * 60) / RASTER_MIN;

/** Stufen des Ablaufs. */
export type Stufe = "waehlen" | "sammeln" | "entscheiden";

export function stufeVon(poll: PollDefinition | null): Stufe {
  if (!poll || poll.status === "entwurf") return "waehlen";
  if (poll.status === "offen") return "sammeln";
  return "entscheiden";
}

/** Montag 00:00 Ortszeit der Woche, in der `ms` liegt. */
export function wochenbeginn(ms: number): number {
  const d = new Date(ms);
  const wt = (d.getDay() + 6) % 7;
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() - wt).getTime();
}

/** Zeitpunkt von Tag `tag` (0 = Montag) und Rasterzeile `zeile` einer Woche. */
export function zellenZeit(woche: number, tag: number, zeile: number): number {
  const d = new Date(woche);
  return new Date(
    d.getFullYear(),
    d.getMonth(),
    d.getDate() + tag,
    TAG_VON,
    zeile * RASTER_MIN,
  ).getTime();
}

const zwei = (n: number) => String(n).padStart(2, "0");

/** `YYYY-MM-DDTHH:MM:SS` in Ortszeit, wie `ScheduleWindow` es erwartet. */
export function lokalIso(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${zwei(d.getMonth() + 1)}-${zwei(d.getDate())}T${zwei(
    d.getHours(),
  )}:${zwei(d.getMinutes())}:00`;
}

/** Abfragefenster Montag 00:00 bis Samstag 00:00 der Woche (240 Intervalle à 30 min). */
export function wochenFenster(woche: number, zeitzone: string) {
  const d = new Date(woche);
  const ende = new Date(
    d.getFullYear(),
    d.getMonth(),
    d.getDate() + 5,
  ).getTime();
  return { start: lokalIso(woche), end: lokalIso(ende), time_zone: zeitzone };
}

/** Index des 30-min-Intervalls im Wochenfenster für eine Zelle. */
export function intervallIndex(
  woche: number,
  tag: number,
  zeile: number,
): number {
  return Math.round(
    (zellenZeit(woche, tag, zeile) - woche) / (RASTER_MIN * MIN),
  );
}

/** Heatmap-Stufe einer Zelle: 0 = unbekannt, 1 = Pflicht-Person belegt, 2 = vorläufig belegt
 *  oder höchstens die Hälfte frei, 3 = die meisten frei, 5 = alle frei. */
export type Heat = 0 | 1 | 2 | 3 | 5;

export interface ZellStand {
  heat: Heat;
  frei: number;
  bekannt: number;
  /** Personen ohne Kalenderauskunft; dann nie „alle frei“. */
  unbekannt: number;
  /** Adressen mit Belegung (nur Anzahl wird angezeigt, keine Termininhalte). */
  belegt: string[];
}

const FREI: Availability[] = ["free", "workingElsewhere"];

export function zellStand(
  verfuegbar: PersonAvailability[],
  pflicht: Set<string>,
  index: number,
  dauerIntervalle = 1,
): ZellStand {
  let frei = 0;
  let bekannt = 0;
  let unbekannt = 0;
  let vorlaeufig = false;
  let pflichtBelegt = false;
  const belegt: string[] = [];
  for (const p of verfuegbar) {
    const werte = p.intervals.slice(index, index + dauerIntervalle);
    if (werte.length < dauerIntervalle || werte.some((w) => w === "unknown")) {
      unbekannt += 1;
      continue;
    }
    bekannt += 1;
    if (werte.every((w) => FREI.includes(w))) {
      frei += 1;
      continue;
    }
    belegt.push(p.address);
    if (werte.every((w) => FREI.includes(w) || w === "tentative"))
      vorlaeufig = true;
    else if (pflicht.has(p.address.toLowerCase())) pflichtBelegt = true;
  }
  const heat: Heat =
    bekannt === 0
      ? 0
      : pflichtBelegt
        ? 1
        : frei === bekannt && unbekannt === 0
          ? 5
          : !vorlaeufig && frei * 2 > bekannt
            ? 3
            : 2;
  return { heat, frei, bekannt, unbekannt, belegt };
}

/** Jüngste Stimme je Person und Vorschlag (wie das Crate; Anzeige). */
export function aktuelleAntworten(antworten: Antwort[]): Map<string, Antwort> {
  const m = new Map<string, Antwort>();
  for (const a of antworten) {
    const k = `${a.person}|${a.vorschlag_nr}`;
    const alt = m.get(k);
    if (!alt || alt.eingegangen <= a.eingegangen) m.set(k, a);
  }
  return m;
}

export type Zelle = {
  wert: Wert;
  quelle: Antwort["quelle"] | null;
  sicher: boolean;
};

export function zelle(
  aktuell: Map<string, Antwort>,
  person: string,
  nr: number,
): Zelle {
  const a = aktuell.get(`${person.toLowerCase()}|${nr}`);
  if (!a) return { wert: "offen", quelle: null, sicher: true };
  return { wert: a.wert, quelle: a.quelle, sicher: a.sicher };
}

/** Zähler je Vorschlag für die Fußzeile der Matrix. */
export function summe(
  aktuell: Map<string, Antwort>,
  personen: Person[],
  nr: number,
): { passt: number; notfalls: number; geantwortet: number } {
  let passt = 0;
  let notfalls = 0;
  let geantwortet = 0;
  for (const p of personen) {
    const z = zelle(aktuell, p.adresse, nr);
    if (z.wert === "offen" || !z.sicher) continue;
    geantwortet += 1;
    if (z.wert === "passt") passt += 1;
    else if (z.wert === "notfalls") notfalls += 1;
  }
  return { passt, notfalls, geantwortet };
}

/** Personen, die noch keine sichere Antwort zu irgendeinem Vorschlag haben. */
export function offenePersonen(
  poll: PollDefinition,
  antworten: Antwort[],
): Person[] {
  const hat = new Set(
    antworten.filter((a) => a.sicher).map((a) => a.person.toLowerCase()),
  );
  return poll.teilnehmende.filter((p) => !hat.has(p.adresse.toLowerCase()));
}

/** Einträge „zur Prüfung“: erkannte, aber nicht sichere Antworten ohne Bestätigung. */
export function zurPruefung(verlauf: PollHistory[]): PollHistory[] {
  return verlauf.filter(
    (h) =>
      !h.confirmed &&
      (h.result.art === "pruefen" || !h.result.sicher) &&
      h.result.art !== "ignorieren",
  );
}

export function initialen(name: string, adresse: string): string {
  const teile = (name || adresse.split("@")[0])
    .split(/[\s._-]+/)
    .filter(Boolean);
  const a = teile[0]?.[0] ?? "?";
  const b = teile.length > 1 ? teile[teile.length - 1][0] : "";
  return (a + b).toUpperCase();
}

export function adresseGueltig(s: string): boolean {
  return /^[^\s@<>()",;]+@[^\s@<>()",;]+\.[^\s@<>()",;]+$/.test(s.trim());
}

/** Neuer Vorschlag aus einer Rasterzelle; Nummern bleiben lückenlos 1..n. */
export function vorschlagUmschalten(
  liste: Vorschlag[],
  start: number,
  dauerMin: number,
  max = 8,
): Vorschlag[] {
  const ohne = liste.filter((v) => v.start !== start);
  const neu =
    ohne.length !== liste.length
      ? ohne
      : ohne.length >= max
        ? ohne
        : [...ohne, { nr: 0, start, ende: start + dauerMin * MIN }];
  return neu
    .slice()
    .sort((a, b) => a.start - b.start)
    .map((v, i) => ({
      ...v,
      nr: i + 1,
      ortszeit: v.ortszeit ?? ortszeitVon(v.start),
    }));
}

/** Ortszeit eines Zeitpunkts mit Versatz je Vorschlag (Sommer-/Winterzeit). */
export function ortszeitVon(ms: number): Ortszeit {
  const d = new Date(ms);
  return {
    jahr: d.getFullYear(),
    monat: d.getMonth() + 1,
    tag: d.getDate(),
    wochentag: d.getDay() || 7,
    stunde: d.getHours(),
    minute: d.getMinutes(),
    utc_versatz_min: -d.getTimezoneOffset(),
  };
}

export function istIntern(
  adresse: string,
  eigeneDomain: string | null,
): boolean {
  if (!eigeneDomain) return false;
  return adresse.toLowerCase().endsWith(`@${eigeneDomain.toLowerCase()}`);
}
