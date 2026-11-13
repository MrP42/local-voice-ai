/**
 * Auswertung am Gesprächsende (lva#559, Paket G5-Kern): Textnormalisierung für Vergleiche, Zahlwörter und die Fristauflösung.
 * Reine Funktionen ohne Zeitzonenlogik (`heute` kommt als Jahr/Monat/Tag), deutsch und englisch aus einer festen Liste.
 * Gegenstück: `AssistantOutcomeDates.swift`; die gemeinsamen Prüfstände stehen in `apps/shared/assistenten/auswertung-pruefstaende.json`.
 */
import { codepunkte } from "./profil";

/** Kalendertag ohne Zeitzone. */
export type Datum = { jahr: number; monat: number; tag: number };

// ---------------------------------------------------------------- Kalender

export const istSchaltjahr = (j: number): boolean =>
  (j % 4 === 0 && j % 100 !== 0) || j % 400 === 0;

export function tageImMonat(jahr: number, monat: number): number {
  if (monat === 2) return istSchaltjahr(jahr) ? 29 : 28;
  return [4, 6, 9, 11].includes(monat) ? 30 : 31;
}

export function istGueltigesDatum(d: Datum): boolean {
  return (
    Number.isInteger(d.jahr) &&
    Number.isInteger(d.monat) &&
    Number.isInteger(d.tag) &&
    d.jahr >= 1000 &&
    d.jahr <= 9999 &&
    d.monat >= 1 &&
    d.monat <= 12 &&
    d.tag >= 1 &&
    d.tag <= tageImMonat(d.jahr, d.monat)
  );
}

/** Tage seit 1970-01-01 (proleptisch gregorianisch). */
export function tagNummer(d: Datum): number {
  const y = d.monat <= 2 ? d.jahr - 1 : d.jahr;
  const era = Math.floor(y / 400);
  const yoe = y - era * 400;
  const doy =
    Math.floor((153 * (d.monat + (d.monat > 2 ? -3 : 9)) + 2) / 5) + d.tag - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146097 + doe - 719468;
}

export function datumAusTagNummer(n: number): Datum {
  const z = n + 719468;
  const era = Math.floor(z / 146097);
  const doe = z - era * 146097;
  const yoe = Math.floor(
    (doe -
      Math.floor(doe / 1460) +
      Math.floor(doe / 36524) -
      Math.floor(doe / 146096)) /
      365,
  );
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
  const mp = Math.floor((5 * doy + 2) / 153);
  const tag = doy - Math.floor((153 * mp + 2) / 5) + 1;
  const monat = mp < 10 ? mp + 3 : mp - 9;
  return { jahr: yoe + era * 400 + (monat <= 2 ? 1 : 0), monat, tag };
}

/** 1 = Montag ... 7 = Sonntag (wie `PlatzhalterWerte.wochentag`). */
export function wochentag(d: Datum): number {
  return ((((tagNummer(d) + 3) % 7) + 7) % 7) + 1;
}

const ziffern = (n: number, stellen: number) =>
  String(n).padStart(stellen, "0");

/** `JJJJ-MM-TT` */
export function formatiereDatum(d: Datum): string {
  return `${ziffern(d.jahr, 4)}-${ziffern(d.monat, 2)}-${ziffern(d.tag, 2)}`;
}

function nurZiffern(s: string): boolean {
  return s.length > 0 && /^[0-9]+$/.test(s);
}

/** Strenges `JJJJ-MM-TT` mit gültigem Kalendertag; sonst `null`. */
export function parseIsoDatum(s: string): Datum | null {
  if (s.length !== 10 || s[4] !== "-" || s[7] !== "-") return null;
  const j = s.slice(0, 4);
  const m = s.slice(5, 7);
  const t = s.slice(8, 10);
  if (!nurZiffern(j) || !nurZiffern(m) || !nurZiffern(t)) return null;
  const d = { jahr: Number(j), monat: Number(m), tag: Number(t) };
  return istGueltigesDatum(d) ? d : null;
}

// ---------------------------------------------------------------- Normalisierung

const LEERRAUM = new Set([0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0xa0]);
const ANFUEHRUNG = new Set([
  0x22, 0x27, 0x60, 0xb4, 0xab, 0xbb, 0x2018, 0x2019, 0x201a, 0x201b, 0x201c,
  0x201d, 0x201e, 0x201f, 0x2039, 0x203a,
]);
const RAND = new Set([
  0x2e, 0x2c, 0x3b, 0x3a, 0x21, 0x3f, 0x2026, 0x28, 0x29, 0x5b, 0x5d, 0x7b,
  0x7d, 0x2d, 0x2013, 0x2014,
]);

export const istLeerraum = (c: number): boolean => LEERRAUM.has(c);

const BLOCK = 8192;

/** Text aus Codepunkten `von`..`bis`, in Blöcken (ein einziger Spread über ein langes Feld wirft `RangeError`). */
export function ausCodepunkten(
  c: readonly number[],
  von = 0,
  bis = c.length,
): string {
  let aus = "";
  for (let i = von; i < bis; i += BLOCK) {
    aus += String.fromCodePoint(...c.slice(i, Math.min(i + BLOCK, bis)));
  }
  return aus;
}

function kleinbuchstabe(c: number): number[] {
  if (c < 0x80) return [c >= 0x41 && c <= 0x5a ? c + 32 : c];
  return codepunkte(String.fromCodePoint(c).toLowerCase());
}

/**
 * Normalisierung für Vergleiche: Kleinschreibung (je Codepunkt), Anführungszeichen entfernen, Leerraum (U+0009–U+000D, U+0020,
 * U+00A0) zu einem Leerzeichen zusammenfassen, Leerraum und Satzzeichen (`. , ; : ! ? … ( ) [ ] { } - – —`) an den Rändern
 * entfernen. Innen bleiben Satzzeichen stehen.
 */
export function normalisiere(s: string): string {
  const aus: number[] = [];
  for (const c of codepunkte(s)) {
    if (ANFUEHRUNG.has(c)) continue;
    if (LEERRAUM.has(c)) {
      if (aus.length > 0 && aus[aus.length - 1] !== 0x20) aus.push(0x20);
      continue;
    }
    for (const k of kleinbuchstabe(c)) aus.push(k);
  }
  let start = 0;
  let ende = aus.length;
  const rand = (c: number) => c === 0x20 || RAND.has(c);
  while (start < ende && rand(aus[start])) start += 1;
  while (ende > start && rand(aus[ende - 1])) ende -= 1;
  return ausCodepunkten(aus, start, ende);
}

/** Entfernt Leerraum an den Enden (dieselbe Menge wie bei der Normalisierung). */
export function kappeLeerraum(s: string): string {
  const c = codepunkte(s);
  let start = 0;
  let ende = c.length;
  while (start < ende && LEERRAUM.has(c[start])) start += 1;
  while (ende > start && LEERRAUM.has(c[ende - 1])) ende -= 1;
  return ausCodepunkten(c, start, ende);
}

/** Fasst Leerraum zu einem Leerzeichen zusammen und kappt die Enden (für einzeilige Texte im Prompt). */
export function einzeilig(s: string): string {
  const aus: number[] = [];
  for (const c of codepunkte(s)) {
    if (LEERRAUM.has(c)) {
      if (aus.length > 0 && aus[aus.length - 1] !== 0x20) aus.push(0x20);
    } else {
      aus.push(c);
    }
  }
  while (aus.length > 0 && aus[aus.length - 1] === 0x20) aus.pop();
  return ausCodepunkten(aus);
}

const istBuchstabe = (c: number) =>
  (c >= 0x61 && c <= 0x7a) ||
  c === 0xe4 ||
  c === 0xf6 ||
  c === 0xfc ||
  c === 0xdf;
const istZiffer = (c: number) => c >= 0x30 && c <= 0x39;

/** Wörter eines normalisierten Textes: Folgen aus `a–z ä ö ü ß`. */
export function woerter(normalisiert: string): string[] {
  const aus: string[] = [];
  let aktuell: number[] = [];
  for (const c of codepunkte(normalisiert)) {
    if (istBuchstabe(c)) {
      aktuell.push(c);
    } else if (aktuell.length > 0) {
      aus.push(ausCodepunkten(aktuell));
      aktuell = [];
    }
  }
  if (aktuell.length > 0) aus.push(ausCodepunkten(aktuell));
  return aus;
}

// ---------------------------------------------------------------- Zahlwörter

/** Zahlwörter null bis zwölf, deutsch und englisch (`ein`-Formen zählen als 1). */
export const ZAHLWOERTER: Readonly<Record<string, number>> = {
  null: 0,
  zero: 0,
  ein: 1,
  eins: 1,
  eine: 1,
  einen: 1,
  einem: 1,
  einer: 1,
  eines: 1,
  one: 1,
  zwei: 2,
  two: 2,
  drei: 3,
  three: 3,
  vier: 4,
  four: 4,
  fünf: 5,
  fuenf: 5,
  five: 5,
  sechs: 6,
  six: 6,
  sieben: 7,
  seven: 7,
  acht: 8,
  eight: 8,
  neun: 9,
  nine: 9,
  zehn: 10,
  ten: 10,
  elf: 11,
  eleven: 11,
  zwölf: 12,
  zwoelf: 12,
  twelve: 12,
};

function zahlwort(wort: string): number | null {
  return Object.prototype.hasOwnProperty.call(ZAHLWOERTER, wort)
    ? ZAHLWOERTER[wort]
    : null;
}

// ---------------------------------------------------------------- Fristauflösung

const MONATE: Readonly<Record<string, number>> = {
  januar: 1,
  january: 1,
  februar: 2,
  february: 2,
  märz: 3,
  maerz: 3,
  march: 3,
  april: 4,
  mai: 5,
  may: 5,
  juni: 6,
  june: 6,
  juli: 7,
  july: 7,
  august: 8,
  september: 9,
  oktober: 10,
  october: 10,
  november: 11,
  dezember: 12,
  december: 12,
};
const WOCHENTAGE: Readonly<Record<string, number>> = {
  montag: 1,
  monday: 1,
  dienstag: 2,
  tuesday: 2,
  mittwoch: 3,
  wednesday: 3,
  donnerstag: 4,
  thursday: 4,
  freitag: 5,
  friday: 5,
  samstag: 6,
  sonnabend: 6,
  saturday: 6,
  sonntag: 7,
  sunday: 7,
};
const VORSAETZE = new Set([
  "am",
  "bis",
  "zum",
  "zur",
  "spätestens",
  "spaetestens",
  "by",
  "until",
  "on",
]);
const NAECHSTE = new Set([
  "nächsten",
  "naechsten",
  "nächster",
  "naechster",
  "kommenden",
  "next",
]);
const UEBERNAECHSTE = new Set([
  "übernächsten",
  "uebernaechsten",
  "übernächster",
  "uebernaechster",
]);
const TAGE_EINHEIT = new Set(["tag", "tage", "tagen", "day", "days"]);
const WOCHEN_EINHEIT = new Set(["woche", "wochen", "week", "weeks"]);

function suche<T>(
  tabelle: Readonly<Record<string, T>>,
  schluessel: string,
): T | null {
  return Object.prototype.hasOwnProperty.call(tabelle, schluessel)
    ? tabelle[schluessel]
    : null;
}

/** Datum ab `heute` plus `tage`; `null` außerhalb der Jahre 1000–9999. */
function plusTage(heute: Datum, tage: number): string | null {
  const d = datumAusTagNummer(tagNummer(heute) + tage);
  return istGueltigesDatum(d) ? formatiereDatum(d) : null;
}

/** Nächstes Vorkommen von Monat/Tag ab `heute` (heute eingeschlossen); 29.2. sucht das nächste Schaltjahr. */
function naechstesVorkommen(
  heute: Datum,
  monat: number,
  tag: number,
): string | null {
  const ab = tagNummer(heute);
  for (let jahr = heute.jahr; jahr <= heute.jahr + 8; jahr += 1) {
    const d = { jahr, monat, tag };
    if (istGueltigesDatum(d) && tagNummer(d) >= ab) return formatiereDatum(d);
  }
  return null;
}

function mitJahr(jahr: number, monat: number, tag: number): string | null {
  const d = { jahr, monat, tag };
  return istGueltigesDatum(d) ? formatiereDatum(d) : null;
}

/** `14`, `14.`, `14th`, `14,` (Tag eines Datums mit Monatsnamen); sonst `null`. */
function tagToken(t: string): number | null {
  let s = t;
  if (s.endsWith(",") || s.endsWith(".")) s = s.slice(0, -1);
  for (const endung of ["st", "nd", "rd", "th"]) {
    if (s.endsWith(endung)) {
      s = s.slice(0, -2);
      break;
    }
  }
  if (!nurZiffern(s) || s.length > 2) return null;
  return Number(s);
}

function jahrToken(t: string): number | null {
  return nurZiffern(t) && t.length === 4 ? Number(t) : null;
}

function datumMitMonatsname(tokens: string[], heute: Datum): string | null {
  // `14. November [2026]`, `14 November`, `14th of November`
  const a = tokens;
  const tagA = a.length >= 2 ? tagToken(a[0]) : null;
  if (tagA !== null) {
    let i = 1;
    if (a[i] === "of") i += 1;
    const monat = i < a.length ? suche(MONATE, a[i]) : null;
    if (monat !== null) {
      const rest = a.slice(i + 1);
      if (rest.length === 0) return naechstesVorkommen(heute, monat, tagA);
      const jahr = rest.length === 1 ? jahrToken(rest[0]) : null;
      return jahr === null ? null : mitJahr(jahr, monat, tagA);
    }
  }
  // `November 14[th][, 2026]`
  const monat = suche(MONATE, a[0]);
  if (monat !== null && a.length >= 2) {
    const tag = tagToken(a[1]);
    if (tag === null) return null;
    const rest = a.slice(2);
    if (rest.length === 0) return naechstesVorkommen(heute, monat, tag);
    const jahr = rest.length === 1 ? jahrToken(rest[0]) : null;
    return jahr === null ? null : mitJahr(jahr, monat, tag);
  }
  return null;
}

function zahlDatum(t: string, heute: Datum): string | null | undefined {
  // `2026-11-14`
  if (t.length === 10 && t[4] === "-") {
    const d = parseIsoDatum(t);
    return d ? formatiereDatum(d) : null;
  }
  // `14.11.`, `14.11`, `14.11.2026`
  const teile = t.split(".");
  if (teile.length < 2 || teile.length > 3) return undefined;
  if (!teile.every(nurZiffern)) return undefined;
  if (teile[0].length > 2 || teile[1].length > 2) return undefined;
  const tag = Number(teile[0]);
  const monat = Number(teile[1]);
  if (teile.length === 2) return naechstesVorkommen(heute, monat, tag);
  return teile[2].length === 4 ? mitJahr(Number(teile[2]), monat, tag) : null;
}

/**
 * Löst eine Fristangabe zu `JJJJ-MM-TT` auf; alles außerhalb der festen Liste ist `null`. Der Ausdruck wird normalisiert, führende
 * Vorsätze (`am`, `bis`, `zum`, `zur`, `spätestens`, `by`, `until`, `on`) entfallen. Wochentage meinen den nächsten solchen Tag
 * nach `heute` (`übernächsten`: eine Woche später), `Ende der Woche` den Freitag der laufenden Woche (am Wochenende den der
 * folgenden), `nächste Woche` den Montag der Folgewoche, Datumsangaben ohne Jahr das nächste Vorkommen ab `heute`.
 */
export function frist(ausdruck: string, heute: Datum): string | null {
  if (!istGueltigesDatum(heute)) return null;
  const alle = normalisiere(ausdruck)
    .split(" ")
    .filter((t) => t !== "");
  let ab = 0;
  while (alle.length - ab > 1 && VORSAETZE.has(alle[ab])) ab += 1;
  const tokens = alle.slice(ab);
  if (tokens.length === 0 || tokens.length > 6) return null;
  const text = tokens.join(" ");
  const heuteWt = wochentag(heute);

  switch (text) {
    case "heute":
    case "today":
      return plusTage(heute, 0);
    case "morgen":
    case "tomorrow":
      return plusTage(heute, 1);
    case "übermorgen":
    case "uebermorgen":
    case "day after tomorrow":
      return plusTage(heute, 2);
    case "ende der woche":
    case "ende dieser woche":
    case "end of the week":
    case "end of this week":
      return plusTage(heute, heuteWt <= 5 ? 5 - heuteWt : 12 - heuteWt);
    case "ende des monats":
    case "ende dieses monats":
    case "end of the month":
    case "end of this month":
      return mitJahr(
        heute.jahr,
        heute.monat,
        tageImMonat(heute.jahr, heute.monat),
      );
    case "nächste woche":
    case "naechste woche":
    case "next week":
      return plusTage(heute, 8 - heuteWt);
    default:
  }

  const wt = (t: string) => suche(WOCHENTAGE, t);
  const bisWochentag = (ziel: number) => ((ziel - heuteWt + 6) % 7) + 1;
  if (tokens.length === 1 && wt(tokens[0]) !== null) {
    return plusTage(heute, bisWochentag(wt(tokens[0])!));
  }
  if (tokens.length === 2 && wt(tokens[1]) !== null) {
    if (NAECHSTE.has(tokens[0]))
      return plusTage(heute, bisWochentag(wt(tokens[1])!));
    if (UEBERNAECHSTE.has(tokens[0]))
      return plusTage(heute, bisWochentag(wt(tokens[1])!) + 7);
  }

  if (tokens.length === 3 && tokens[0] === "in") {
    const zahl =
      nurZiffern(tokens[1]) && tokens[1].length <= 3
        ? Number(tokens[1])
        : zahlwort(tokens[1]);
    if (zahl !== null) {
      if (TAGE_EINHEIT.has(tokens[2])) return plusTage(heute, zahl);
      if (WOCHEN_EINHEIT.has(tokens[2])) return plusTage(heute, zahl * 7);
    }
    return null;
  }

  if (tokens.length === 1) {
    const d = zahlDatum(tokens[0], heute);
    return d === undefined ? null : d;
  }
  return datumMitMonatsname(tokens, heute);
}

/** `fristenIn` untersucht höchstens die ersten so vielen Wörter des Textes. */
export const MAX_FRIST_WOERTER = 1024;

/**
 * Alle Tage, die eine Folge von bis zu 6 Wörtern des (normalisierten) Textes als Frist auflöst; Reihenfolge des Auftretens, ohne
 * Wiederholung. Untersucht werden höchstens die ersten `MAX_FRIST_WOERTER` Wörter.
 */
export function fristenIn(text: string, heute: Datum): string[] {
  const tokens = normalisiere(text)
    .split(" ")
    .filter((t) => t !== "")
    .slice(0, MAX_FRIST_WOERTER);
  const aus: string[] = [];
  for (let i = 0; i < tokens.length; i += 1) {
    for (let n = 1; n <= 6 && i + n <= tokens.length; n += 1) {
      const d = frist(tokens.slice(i, i + n).join(" "), heute);
      if (d !== null && !aus.includes(d)) aus.push(d);
    }
  }
  return aus;
}

// ---------------------------------------------------------------- Zahlen

/** Vergleichsform einer Zahl: ohne führende Nullen vorn und nachlaufende Nullen im Bruch, ohne Vorzeichen. */
function vergleichsform(ganz: string, bruch: string): string {
  const g = ganz.replace(/^0+(?=[0-9])/, "");
  const b = bruch.replace(/0+$/, "");
  return b === "" ? g : `${g}.${b}`;
}

/**
 * Kanonische Schreibweise einer Zahl als Text (Punkt als Dezimaltrenner, ohne Tausenderzeichen) und ihre Vergleichsform.
 * Gelesen werden `-?\d+`, `-?\d+[.,]\d+`, `1.250,50`, `1,250.50` und Zahlwörter null bis zwölf; sonst `null`.
 */
export function liesZahl(
  s: string,
): { kanonisch: string; vergleich: string } | null {
  const t = kappeLeerraum(s);
  const vorzeichen = t.startsWith("-") ? "-" : "";
  const k = vorzeichen ? t.slice(1) : t;
  const einfach = /^([0-9]+)(?:[.,]([0-9]+))?$/.exec(k);
  if (einfach) {
    const bruch = einfach[2] ?? "";
    return {
      kanonisch: `${vorzeichen}${einfach[1]}${bruch === "" ? "" : `.${bruch}`}`,
      vergleich: vergleichsform(einfach[1], bruch),
    };
  }
  const deutsch = /^([0-9]{1,3}(?:\.[0-9]{3})+),([0-9]+)$/.exec(k);
  if (deutsch) {
    const ganz = deutsch[1].replace(/\./g, "");
    return {
      kanonisch: `${vorzeichen}${ganz}.${deutsch[2]}`,
      vergleich: vergleichsform(ganz, deutsch[2]),
    };
  }
  const englisch = /^([0-9]{1,3}(?:,[0-9]{3})+)\.([0-9]+)$/.exec(k);
  if (englisch) {
    const ganz = englisch[1].replace(/,/g, "");
    return {
      kanonisch: `${vorzeichen}${ganz}.${englisch[2]}`,
      vergleich: vergleichsform(ganz, englisch[2]),
    };
  }
  const wort = vorzeichen === "" ? zahlwort(normalisiere(t)) : null;
  if (wort !== null)
    return { kanonisch: String(wort), vergleich: String(wort) };
  return null;
}

/** Vergleichsformen der Zahlen eines normalisierten Textes (Ziffernfolgen, durch ein einzelnes `.` oder `,` zwischen Ziffern verbunden). */
export function zahlenIn(normalisiert: string): string[] {
  const c = codepunkte(normalisiert);
  const aus: string[] = [];
  let i = 0;
  while (i < c.length) {
    if (!istZiffer(c[i])) {
      i += 1;
      continue;
    }
    const gruppen: string[] = [];
    const trenner: number[] = [];
    for (;;) {
      let j = i;
      while (j < c.length && istZiffer(c[j])) j += 1;
      gruppen.push(ausCodepunkten(c, i, j));
      i = j;
      if (
        i + 1 < c.length &&
        (c[i] === 0x2e || c[i] === 0x2c) &&
        istZiffer(c[i + 1])
      ) {
        trenner.push(c[i]);
        i += 1;
      } else {
        break;
      }
    }
    for (const form of zahlKandidaten(gruppen, trenner)) {
      if (!aus.includes(form)) aus.push(form);
    }
  }
  return aus;
}

function zahlKandidaten(gruppen: string[], trenner: number[]): string[] {
  if (gruppen.length === 1) return [vergleichsform(gruppen[0], "")];
  const tausender = (teile: string[]) =>
    teile[0].length >= 1 &&
    teile[0].length <= 3 &&
    teile.slice(1).every((g) => g.length === 3);
  const aus: string[] = [];
  const letzter = trenner[trenner.length - 1];
  const gleich = trenner.every((t) => t === trenner[0]);
  if (gleich) {
    if (gruppen.length === 2) aus.push(vergleichsform(gruppen[0], gruppen[1]));
    if (tausender(gruppen)) aus.push(vergleichsform(gruppen.join(""), ""));
    return aus;
  }
  // gemischte Trenner: der letzte ist das Komma des Bruchs, davor stehen Tausendergruppen mit dem anderen Zeichen
  const davor = gruppen.slice(0, -1);
  const andere = trenner.slice(0, -1);
  if (
    andere.every((t) => t === andere[0] && t !== letzter) &&
    tausender(davor)
  ) {
    aus.push(vergleichsform(davor.join(""), gruppen[gruppen.length - 1]));
  }
  return aus;
}

/** Hat der normalisierte Text ein Zahlwort (null bis zwölf), das für `zahl` steht? */
export function hatZahlwort(normalisiert: string, zahl: number): boolean {
  return woerter(normalisiert).some((w) => zahlwort(w) === zahl);
}
