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

/** Zeilentrenner, die im Prompt nie stehen dürfen: Leerraum der Normalisierung plus U+0085, U+2028, U+2029. */
const ZEILENTRENNER = new Set([...LEERRAUM, 0x85, 0x2028, 0x2029]);

/**
 * Fasst Leerraum und Zeilentrenner (U+0009–U+000D, U+0020, U+0085, U+00A0, U+2028, U+2029) zu einem Leerzeichen zusammen und kappt
 * die Enden (für einzeilige Texte im Prompt).
 */
export function einzeilig(s: string): string {
  const aus: number[] = [];
  for (const c of codepunkte(s)) {
    if (ZEILENTRENNER.has(c)) {
      if (aus.length > 0 && aus[aus.length - 1] !== 0x20) aus.push(0x20);
    } else {
      aus.push(c);
    }
  }
  while (aus.length > 0 && aus[aus.length - 1] === 0x20) aus.pop();
  return ausCodepunkten(aus);
}

const istZiffer = (c: number) => c >= 0x30 && c <= 0x39;

/**
 * Wortzeichen eines normalisierten Textes: `a–z`, `0–9` und jeder Codepunkt ab U+0080 außer U+00A0–U+00BF (Zeichen und Satzzeichen
 * des Latin-1-Blocks) und U+2000–U+206F (allgemeine Zeichensetzung). Alles andere trennt Wörter.
 */
export function istWortzeichen(c: number): boolean {
  if (c >= 0x61 && c <= 0x7a) return true;
  if (istZiffer(c)) return true;
  if (c < 0x80) return false;
  return !(c >= 0xa0 && c <= 0xbf) && !(c >= 0x2000 && c <= 0x206f);
}

/** Wörter eines normalisierten Textes: maximale Folgen von Wortzeichen. */
export function woerter(normalisiert: string): string[] {
  const aus: string[] = [];
  let aktuell: number[] = [];
  for (const c of codepunkte(normalisiert)) {
    if (istWortzeichen(c)) {
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

/** `fristenIn` untersucht Ausdrücke, die in den ersten so vielen Wörtern eines Textes beginnen. */
export const MAX_FRIST_WOERTER = 1024;

/** Größtes Fenster (in Wörtern), das die Fristauflösung betrachtet. */
export const MAX_FRIST_FENSTER = 6;

/** Höchstens so viele Fundstellen eines Belegs in einem Beitrag werden für die Deckung geprüft; mehr: nicht gedeckt. */
export const MAX_FUNDSTELLEN = 16;

/**
 * Rand eines Fensters: So viele Wörter müssen hinter (und vor) einem Fenster gelesen sein, damit seine Nachbarschaftsprüfung
 * vollständig ist, sofern die Wortliste dort abgeschnitten ist. Hinter einem Fenster liest die Prüfung höchstens 5 Wörter: bis zu
 * 2 Brückenwörter, `in`, ein Zahlwort und dessen Folgewort. Ein längeres auflösendes Fenster (höchstens 6 Wörter) reicht ebenfalls
 * höchstens 5 Wörter über ein kürzeres hinaus. Davor gilt es symmetrisch. Am echten Ende des Textes gilt der Rand nicht.
 */
export const FRIST_RAND = 5;

/**
 * Nachbarschaft einer Fundstelle: so viele Wörter davor und danach gehören zum Kontext (ein Fenster, das die Fundstelle schneidet,
 * reicht bis zu 5 Wörter über sie hinaus und hat dann noch `FRIST_RAND` Wörter Rand).
 */
export const KONTEXT_WOERTER = 12;

/**
 * Dafür werden je Seite höchstens so viele Codepunkte gelesen. Reicht das nicht und der Beitrag geht weiter, endet die
 * Nachbarschaft am letzten ganzen Wort davor und gilt als abgeschnitten (siehe `FRIST_RAND`); steckt schon das Wort an der
 * Fundstelle darin fest, gibt es keine Nachbarschaft.
 */
export const KONTEXT_CODEPUNKTE = 400;

/** `fristenIn` liest höchstens so viele Wörter: Fenster beginnen in den ersten 1024, reichen höchstens 5 weiter, dahinter 5 Wörter Rand. */
const FRIST_LESEGRENZE =
  MAX_FRIST_WOERTER + (MAX_FRIST_FENSTER - 1) + FRIST_RAND;

/** Ob die Wortliste vorn oder hinten abgeschnitten ist (der Text geht dort weiter). */
export type Abgeschnitten = { vorn: boolean; hinten: boolean };

/**
 * Wörter, die einen Tag verschieben oder näher bestimmen: `nächsten`/`kommenden`/`next` und `übernächsten` in allen Formen, die
 * Einheiten `Tag`, `Woche`, `Monat`, `Jahr` (deutsch und englisch, alle Formen der Liste).
 */
const MODIFIKATOREN = [
  ...NAECHSTE,
  ...UEBERNAECHSTE,
  "nächste",
  "naechste",
  "nächstes",
  "naechstes",
  "nächstem",
  "naechstem",
  "kommende",
  "kommender",
  "kommendes",
  "kommendem",
  "übernächste",
  "uebernaechste",
  "übernächstes",
  "uebernaechstes",
  "übernächstem",
  "uebernaechstem",
  ...TAGE_EINHEIT,
  ...WOCHEN_EINHEIT,
  "monat",
  "monate",
  "monaten",
  "monats",
  "month",
  "months",
  "jahr",
  "jahre",
  "jahren",
  "jahres",
  "year",
  "years",
];

/**
 * Der feste Wortschatz der Fristauflösung (Datumsbestandteile) ohne Zahlwörter: Wochentage, Monate, Tagesworte, Modifikatoren und
 * Einheiten. Führende Vorsätze (`am`, `bis`, …) gehören nicht dazu. Zahlwörter (null bis zwölf, `ein`-Formen, `one`) zählen nur
 * unter der Bedingung von `istDatumsbestandteil`.
 */
const DATUMSWOERTER = new Set<string>([
  ...Object.keys(WOCHENTAGE),
  ...Object.keys(MONATE),
  ...MODIFIKATOREN,
  "heute",
  "today",
  "morgen",
  "tomorrow",
  "übermorgen",
  "uebermorgen",
  "ende",
  "end",
  "dieser",
  "dieses",
  "this",
]);

/**
 * Verbinder und Verneiner: Steht einer direkt hinter einem Fristausdruck (oder direkt davor, führende Vorsätze übersprungen), ist
 * der Tag womöglich nicht gemeint (`Montag oder Dienstag`, `nicht am Montag`).
 */
const VERBINDER = new Set<string>([
  "oder",
  "bzw",
  "beziehungsweise",
  "entweder",
  "weder",
  "noch",
  "nicht",
  "kein",
  "keine",
  "keinen",
  "keinem",
  "keiner",
  "statt",
  "anstatt",
  "außer",
  "ausser",
  "or",
  "either",
  "neither",
  "nor",
  "not",
  "no",
  "instead",
  "except",
]);

/** Wörter hinter einer Zahl, die sie zur Uhrzeit machen (`10 Uhr`, `10 am`); `o'clock` steht normalisiert als `oclock` da. */
const UHRZEIT_NACH = new Set(["uhr", "h", "oclock", "am", "pm"]);

/** Ein Wort ohne die Satzzeichen an seinen Rändern (dieselbe Menge wie bei der Normalisierung). */
function ohneRand(t: string): string {
  let von = 0;
  let bis = t.length;
  while (von < bis && RAND.has(t.charCodeAt(von))) von += 1;
  while (bis > von && RAND.has(t.charCodeAt(bis - 1))) bis -= 1;
  return t.slice(von, bis);
}

/** Reines Zifferntoken: Ziffernfolgen, durch je ein `.`, `,`, `-` oder `/` verbunden (`14`, `14.11.`, `1.250,50`, `2026-11-14`). */
function istZifferntoken(t: string): boolean {
  return /^[0-9]+(?:[.,/-][0-9]+)*$/.test(t);
}

/**
 * Ist das Wort `i` ein Datumsbestandteil des Ausdrucks, an dessen Rand es steht (`danach`: rechts davon, sonst links davon)?
 * - Wörter des festen Wortschatzes (Wochentag, Monat, Modifikator, Einheit, Tagesworte): immer.
 * - Reine Zifferntoken (`14`, `14.11.`): immer.
 * - Zahlwörter (`zwei`, `eine`, `one`): nur, wenn das nächste Wort in derselben Richtung (vom Ausdruck aus das übernächste) zum
 *   Wortschatz gehört oder ein Zifferntoken ist (`Montag zwei Wochen`); `Montag zwei Personen`, `Freitag einen Bericht` bleiben sicher.
 * - `in` vor einer Ziffernfolge oder vor einem Zahlwort, das selbst zählt (`in zwei Wochen`).
 * Ausnahme Uhrzeit: Ein Zahlwort oder Zifferntoken **hinter** dem Ausdruck zählt nicht, wenn `uhr`, `h`, `oclock`, `am` oder `pm`
 * folgt (`Montag 10 Uhr`); `hh:mm` ist kein Zifferntoken.
 */
function istDatumsbestandteil(
  tokens: readonly string[],
  i: number,
  danach: boolean,
): boolean {
  const t = ohneRand(tokens[i]);
  if (t === "") return false;
  const richtung = danach ? 1 : -1;
  const wort = (k: number): string | null =>
    k >= 0 && k < tokens.length ? ohneRand(tokens[k]) : null;
  const zaehltAlsZahl = (k: number, rich: number): boolean => {
    const w = wort(k);
    if (w === null || w === "") return false;
    if (istZifferntoken(w)) return true;
    if (zahlwort(w) === null) return false;
    const n = wort(k + rich);
    return n !== null && (DATUMSWOERTER.has(n) || istZifferntoken(n));
  };
  const zahl = zahlwort(t) !== null || istZifferntoken(t);
  if (zahl && danach && UHRZEIT_NACH.has(wort(i + 1) ?? "")) return false;
  if (istZifferntoken(t) || DATUMSWOERTER.has(t)) return true;
  if (zahlwort(t) !== null) return zaehltAlsZahl(i, richtung);
  if (t !== "in") return false;
  const n = wort(i + 1);
  return n !== null && (nurZiffern(n) || zaehltAlsZahl(i + 1, 1));
}

/**
 * Steht direkt hinter dem Ausdruck (Wörter `von` bis `bis` einschließlich) oder direkt davor (führende Vorsätze übersprungen) ein
 * Verbinder oder Verneiner? Läuft das Überspringen der Vorsätze über den Anfang einer vorn abgeschnittenen Liste hinaus, fehlt das
 * Wort davor: unsicher.
 */
function hatVerbinder(
  tokens: readonly string[],
  von: number,
  bis: number,
  vornAbgeschnitten: boolean,
): boolean {
  if (bis + 1 < tokens.length && VERBINDER.has(ohneRand(tokens[bis + 1]))) {
    return true;
  }
  let davor = von - 1;
  while (davor >= 0 && VORSAETZE.has(ohneRand(tokens[davor]))) davor -= 1;
  if (davor < 0) return vornAbgeschnitten;
  return VERBINDER.has(ohneRand(tokens[davor]));
}

/**
 * Brückenwörter: Wörter zwischen einem Fristausdruck und einem weiteren Datumsbestandteil (`Montag bis Freitag`, `am Montag im Mai`,
 * `Montag den 14. Dezember`, `Montag und Dienstag`). Eine eigene Menge; sie schließt die führenden Vorsätze ein.
 */
const BRUECKEN = new Set<string>([
  "am",
  "an",
  "bis",
  "zum",
  "zur",
  "spätestens",
  "spaetestens",
  "im",
  "in",
  "den",
  "dem",
  "der",
  "des",
  "vom",
  "von",
  "ab",
  "um",
  "gegen",
  "und",
  "sowie",
  "by",
  "until",
  "till",
  "on",
  "to",
  "through",
  "thru",
  "and",
  "of",
  "the",
  "at",
  "from",
]);

/** Höchstens so viele Brückenwörter werden zwischen Ausdruck und Datumsbestandteil übersprungen. */
export const MAX_BRUECKEN = 2;

/**
 * Steht hinter (`danach`) oder vor dem Ausdruck, nach 1 bis `MAX_BRUECKEN` übersprungenen Brückenwörtern, ein Datumsbestandteil?
 * `i` ist das erste Wort neben dem Ausdruck in dieser Richtung. Das direkte Nachbarwort (kein Brückenwort übersprungen) prüft
 * `istDatumsbestandteil` selbst. Die Uhrzeit-Ausnahme gilt auch hinter Brückenwörtern (`Montag um 10 Uhr`).
 */
function bruecke(
  tokens: readonly string[],
  i: number,
  danach: boolean,
): boolean {
  const schritt = danach ? 1 : -1;
  let k = i;
  let uebersprungen = 0;
  while (
    uebersprungen < MAX_BRUECKEN &&
    k >= 0 &&
    k < tokens.length &&
    BRUECKEN.has(ohneRand(tokens[k]))
  ) {
    k += schritt;
    uebersprungen += 1;
  }
  if (uebersprungen === 0 || k < 0 || k >= tokens.length) return false;
  return istDatumsbestandteil(tokens, k, danach);
}

/** Ein inklusionsmaximaler Fristausdruck: Wörter `von` bis `von + laenge`, der Tag und ob ihn ein Datumsbestandteil begleitet. */
export type FristTreffer = {
  von: number;
  laenge: number;
  tag: string;
  unsicher: boolean;
};

/**
 * Die inklusionsmaximalen Fristausdrücke unter den Wörtern `tokens`, die die Wörter `p` bis `q` (einschließlich) schneiden:
 * Unter allen Fenstern von höchstens `MAX_FRIST_FENSTER` Wörtern, die die Fristauflösung auflöst, bleiben die übrig, die kein
 * längeres auflösendes Fenster enthält (egal, ob es `p..q` verlässt). Reihenfolge: nach Anfang, bei gleichem Anfang kürzere zuerst.
 * `unsicher` ist ein Treffer, wenn das Wort direkt davor oder direkt danach ein Datumsbestandteil ist (siehe `istDatumsbestandteil`)
 * oder direkt daneben ein Verbinder oder Verneiner steht (siehe `hatVerbinder`) oder nach höchstens
 * `MAX_BRUECKEN` Brückenwörtern ein Datumsbestandteil folgt (siehe `bruecke`). Ist die Wortliste hinten (vorn) abgeschnitten und
 * liegen hinter (vor) dem Treffer weniger als `FRIST_RAND` Wörter, ist er ebenfalls unsicher: Das fehlende Folgewort darf die Prüfung
 * nicht umgehen. Am echten Textende (`abgeschnitten` falsch) gilt das nicht.
 * Die Treffer können überlappen und auf verschiedene Tage auflösen; die Zerlegung hängt nicht von der Suchrichtung ab.
 */
export function maximaleFristen(
  tokens: readonly string[],
  p: number,
  q: number,
  heute: Datum,
  abgeschnitten: Abgeschnitten = { vorn: false, hinten: false },
): FristTreffer[] {
  const schluessel = (i: number, n: number) => i * (MAX_FRIST_FENSTER + 1) + n;
  const aufgeloest = new Map<number, string>();
  const reihe: { von: number; laenge: number; tag: string }[] = [];
  const von0 = Math.max(0, p - (MAX_FRIST_FENSTER - 1));
  for (let i = von0; i <= q && i < tokens.length; i += 1) {
    const hoechstens = Math.min(MAX_FRIST_FENSTER, tokens.length - i);
    for (let n = 1; n <= hoechstens; n += 1) {
      if (i + n - 1 < p) continue;
      const tag = frist(tokens.slice(i, i + n).join(" "), heute);
      if (tag === null) continue;
      aufgeloest.set(schluessel(i, n), tag);
      reihe.push({ von: i, laenge: n, tag });
    }
  }
  const enthalten = (i: number, n: number): boolean => {
    for (let m = n + 1; m <= MAX_FRIST_FENSTER; m += 1) {
      for (let j = Math.max(0, i + n - m); j <= i; j += 1) {
        if (aufgeloest.has(schluessel(j, m))) return true;
      }
    }
    return false;
  };
  const aus: FristTreffer[] = [];
  for (const t of reihe) {
    if (enthalten(t.von, t.laenge)) continue;
    const davor = t.von - 1;
    const danach = t.von + t.laenge;
    aus.push({
      ...t,
      unsicher:
        (davor >= 0 && istDatumsbestandteil(tokens, davor, false)) ||
        (danach < tokens.length &&
          istDatumsbestandteil(tokens, danach, true)) ||
        hatVerbinder(tokens, t.von, danach - 1, abgeschnitten.vorn) ||
        bruecke(tokens, davor, false) ||
        bruecke(tokens, danach, true) ||
        (abgeschnitten.hinten && tokens.length - danach < FRIST_RAND) ||
        (abgeschnitten.vorn && t.von < FRIST_RAND),
    });
  }
  return aus;
}

/** Die Wörter eines Textes für die Fristsuche (höchstens `FRIST_LESEGRENZE`) und ob der Text danach weitergeht. */
function lesen(text: string): { tokens: string[]; hinten: boolean } {
  const alle = normalisiere(text)
    .split(" ", FRIST_LESEGRENZE + 1)
    .filter((t) => t !== "");
  const hinten = alle.length > FRIST_LESEGRENZE;
  return { tokens: hinten ? alle.slice(0, FRIST_LESEGRENZE) : alle, hinten };
}

/**
 * Wörter eines Textes für die Fristsuche (normalisiert, an Leerzeichen getrennt); höchstens `MAX_FRIST_WOERTER` plus 5 (längstes
 * Fenster) plus `FRIST_RAND`: Fenster beginnen in den ersten 1024 Wörtern und haben dahinter noch den Rand.
 */
export function fristWoerter(text: string): string[] {
  return lesen(text).tokens;
}

/**
 * Alle Tage, die die inklusionsmaximalen, sicheren Fristausdrücke eines Textes auflösen; Reihenfolge des Auftretens, ohne
 * Wiederholung. Untersucht werden Ausdrücke, die in den ersten `MAX_FRIST_WOERTER` Wörtern beginnen; gelesen werden bis zu
 * `FRIST_RAND` Wörter dahinter. Geht der Text über das Gelesene hinaus, ist ein Treffer mit weniger als `FRIST_RAND` Wörtern dahinter
 * unsicher; am echten Textende gelten die normalen Regeln.
 */
export function fristenIn(text: string, heute: Datum): string[] {
  const { tokens, hinten } = lesen(text);
  if (tokens.length === 0) return [];
  const aus: string[] = [];
  const q = Math.min(MAX_FRIST_WOERTER, tokens.length) - 1;
  for (const t of maximaleFristen(tokens, 0, q, heute, {
    vorn: false,
    hinten,
  })) {
    if (!t.unsicher && !aus.includes(t.tag)) aus.push(t.tag);
  }
  return aus;
}

/** Ohne führende Vorsätze (`am`, `bis`, `zum`, …; solange mehr als ein Wort bleibt). */
function ohneVorsaetze(tokens: readonly string[]): readonly string[] {
  let ab = 0;
  while (tokens.length - ab > 1 && VORSAETZE.has(tokens[ab])) ab += 1;
  return tokens.slice(ab);
}

/** Einheitenbreite des Codepunkts `c` in einem JavaScript-Text. */
const breite = (c: number): number => (c > 0xffff ? 2 : 1);

/**
 * Die Fundstellen von `nadel` in `heuhaufen` (beide normalisiert): die Anfänge (Einheiten) aller Vorkommen, die an Wortgrenzen
 * beginnen und enden (vor einem Vorkommen, das mit einem Wortzeichen beginnt, und nach einem, das mit einem endet, steht kein
 * Wortzeichen; `ja` hat in `jahr` keine Fundstelle). Aufsteigend, höchstens `grenze` Stück; Vorkommen dürfen überlappen.
 */
export function fundstellen(
  heuhaufen: string,
  nadel: string,
  grenze: number,
): number[] {
  const aus: number[] = [];
  if (nadel === "" || grenze <= 0) return aus;
  const erster = nadel.codePointAt(0)!;
  const letzter = codepunktVor(nadel, nadel.length);
  const vorn = istWortzeichen(erster);
  const hinten = istWortzeichen(letzter);
  let ab = 0;
  for (;;) {
    const i = heuhaufen.indexOf(nadel, ab);
    if (i < 0) return aus;
    const ende = i + nadel.length;
    const davorOk =
      !vorn || i === 0 || !istWortzeichen(codepunktVor(heuhaufen, i));
    const danachOk =
      !hinten ||
      ende >= heuhaufen.length ||
      !istWortzeichen(heuhaufen.codePointAt(ende)!);
    if (davorOk && danachOk) {
      aus.push(i);
      if (aus.length >= grenze) return aus;
    }
    ab = i + 1;
  }
}

/** Die Wörter (an Leerzeichen getrennt) der Nachbarschaft einer Fundstelle, die Nummern der Wörter, die sie berühren, und die Kürzung. */
type Nachbarschaft = {
  tokens: string[];
  p: number;
  q: number;
  abgeschnitten: Abgeschnitten;
};

/** Die Grenzen (Einheiten) einer Nachbarschaft und ob sie vorn oder hinten gekürzt ist, der Beitrag also dort weitergeht. */
type Grenzen = { von: number; bis: number } & Abgeschnitten;

/**
 * Die Nachbarschaft der Spanne `start`..`ende` (Einheiten) im normalisierten `beitrag`: die Wörter, die die Spanne berühren, plus
 * `KONTEXT_WOERTER` ganze Wörter davor und danach. Je Seite werden höchstens `KONTEXT_CODEPUNKTE` Codepunkte gelesen. Findet sich
 * darin nicht die Grenze zum 13. Wort und der Beitrag geht weiter, endet die Nachbarschaft am letzten ganzen Wort, das gelesen wurde.
 * Jede Seite meldet, ob sie gekürzt ist (durch die Wortgrenze oder die Codepunktgrenze) oder ob dort der Beitrag endet. Steckt das
 * Wort an der Spanne selbst in der Grenze fest (kein Leerzeichen gelesen), gibt es keine Nachbarschaft (`null`).
 */
function nachbarschaft(
  beitrag: string,
  start: number,
  ende: number,
): Grenzen | null {
  let i = start;
  let gelesen = 0;
  let leer = 0;
  let von = -1;
  let aeusserstes = -1;
  let vorn = false;
  while (i > 0 && gelesen < KONTEXT_CODEPUNKTE) {
    const c = codepunktVor(beitrag, i);
    i -= breite(c);
    gelesen += 1;
    if (c === 0x20) {
      leer += 1;
      aeusserstes = i;
      if (leer === KONTEXT_WOERTER + 1) {
        von = i + 1;
        vorn = true;
        break;
      }
    }
  }
  if (von < 0) {
    if (i > 0) {
      if (aeusserstes < 0) return null;
      von = aeusserstes + 1;
      vorn = true;
    } else {
      von = 0;
    }
  }
  i = ende;
  gelesen = 0;
  leer = 0;
  let bis = -1;
  let hinten = false;
  aeusserstes = -1;
  while (i < beitrag.length && gelesen < KONTEXT_CODEPUNKTE) {
    const c = beitrag.codePointAt(i)!;
    i += breite(c);
    gelesen += 1;
    if (c === 0x20) {
      leer += 1;
      aeusserstes = i - 1;
      if (leer === KONTEXT_WOERTER + 1) {
        bis = i - 1;
        hinten = true;
        break;
      }
    }
  }
  if (bis < 0) {
    if (i < beitrag.length) {
      if (aeusserstes < 0) return null;
      bis = aeusserstes;
      hinten = true;
    } else {
      bis = beitrag.length;
    }
  }
  return { von, bis, vorn, hinten };
}

function woerterDerNachbarschaft(
  beitrag: string,
  start: number,
  ende: number,
): Nachbarschaft | null {
  const grenzen = nachbarschaft(beitrag, start, ende);
  if (grenzen === null) return null;
  const tokens = beitrag.slice(grenzen.von, grenzen.bis).split(" ");
  let p = -1;
  let q = -1;
  let anfang = grenzen.von;
  tokens.forEach((t, i) => {
    const schluss = anfang + t.length;
    if (schluss > start && anfang < ende) {
      if (p < 0) p = i;
      q = i;
    }
    anfang = schluss + 1;
  });
  return p < 0
    ? null
    : {
        tokens,
        p,
        q,
        abgeschnitten: { vorn: grenzen.vorn, hinten: grenzen.hinten },
      };
}

/**
 * Der eine Tag, den die Spanne `start`..`ende` (Einheiten) des normalisierten `beitrag` im Kontext nennt, sonst `null`: Unter den
 * inklusionsmaximalen Fristausdrücken der Nachbarschaft, die die Spanne schneiden, ist mindestens einer, keiner ist unsicher, und alle
 * lösen auf denselben Tag auf.
 */
export function fristDeckungImKontext(
  beitrag: string,
  start: number,
  ende: number,
  heute: Datum,
): string | null {
  const n = woerterDerNachbarschaft(beitrag, start, ende);
  if (n === null) return null;
  const treffer = maximaleFristen(n.tokens, n.p, n.q, heute, n.abgeschnitten);
  if (treffer.length === 0 || treffer.some((t) => t.unsicher)) return null;
  const tag = treffer[0].tag;
  return treffer.every((t) => t.tag === tag) ? tag : null;
}

/**
 * Nennt der Beleg `beleg` (normalisiert) im normalisierten `beitrag` den Tag `tag`? An allen Fundstellen (mindestens eine,
 * höchstens `MAX_FUNDSTELLEN`) muss `fristDeckungImKontext` genau diesen Tag liefern.
 */
export function fristGedeckt(
  beitrag: string,
  beleg: string,
  tag: string,
  heute: Datum,
): boolean {
  const stellen = fundstellen(beitrag, beleg, MAX_FUNDSTELLEN + 1);
  if (stellen.length === 0 || stellen.length > MAX_FUNDSTELLEN) return false;
  return stellen.every(
    (s) => fristDeckungImKontext(beitrag, s, s + beleg.length, heute) === tag,
  );
}

/**
 * Steht der Ausdruck `ausdruck` (normalisierte Wörter, bis auf führende Vorsätze) an der Fundstelle `start`..`ende` des normalisierten
 * `beitrag` als ganzer, sicherer, inklusionsmaximaler Fristausdruck?
 */
export function istFristFundstelle(
  beitrag: string,
  start: number,
  ende: number,
  ausdruck: readonly string[],
  heute: Datum,
): boolean {
  const n = woerterDerNachbarschaft(beitrag, start, ende);
  if (n === null) return false;
  const kern = ohneVorsaetze(ausdruck);
  return maximaleFristen(n.tokens, n.p, n.q, heute, n.abgeschnitten).some(
    (t) => {
      if (t.unsicher || t.von > n.p || t.von + t.laenge - 1 < n.q) return false;
      const eigene = ohneVorsaetze(n.tokens.slice(t.von, t.von + t.laenge));
      return (
        eigene.length === kern.length && eigene.every((w, i) => w === kern[i])
      );
    },
  );
}

/**
 * Steht `ausdruck` (Wörter eines `frist_text`) in `beitrag` an einer der ersten `MAX_FUNDSTELLEN` Fundstellen von `text` (normalisiert)
 * als ganzer, sicherer, inklusionsmaximaler Fristausdruck?
 */
export function enthaeltFristAusdruck(
  beitrag: string,
  text: string,
  ausdruck: readonly string[],
  heute: Datum,
): boolean {
  return fundstellen(beitrag, text, MAX_FUNDSTELLEN).some((s) =>
    istFristFundstelle(beitrag, s, s + text.length, ausdruck, heute),
  );
}

// ---------------------------------------------------------------- Zahlen

/** Vergleichsform einer Zahl: ohne führende Nullen vorn und nachlaufende Nullen im Bruch, ohne Vorzeichen. */
function vergleichsform(ganz: string, bruch: string): string {
  const g = ganz.replace(/^0+(?=[0-9])/, "");
  const b = bruch.replace(/0+$/, "");
  return b === "" ? g : `${g}.${b}`;
}

/**
 * Kanonische Schreibweise: optionales `-`, Ganzzahlteil ohne führende Nullen (`0` allein bleibt), optional `.` und Nachkommastellen
 * (unverändert). Ein `-` vor einem Betrag aus lauter Nullen entfällt (`-0` wird `0`).
 */
function kanonischeZahl(
  vorzeichen: string,
  ganz: string,
  bruch: string,
): string {
  const g = ganz.replace(/^0+(?=[0-9])/, "");
  const betragNull = g === "0" && /^0*$/.test(bruch);
  return `${betragNull ? "" : vorzeichen}${g}${bruch === "" ? "" : `.${bruch}`}`;
}

/** Ist `s` die kanonische Schreibweise einer Zahl (siehe `liesZahl`)? Höchstens 500 Zeichen prüft der Aufrufer. */
export function istKanonischeZahl(s: string): boolean {
  const m = /^(-?)(0|[1-9][0-9]*)(?:\.([0-9]+))?$/.exec(s);
  if (m === null) return false;
  return !(m[1] === "-" && m[2] === "0" && /^0*$/.test(m[3] ?? ""));
}

/**
 * Kanonische Schreibweise einer Zahl als Text (Punkt als Dezimaltrenner, ohne Tausenderzeichen, ohne führende Nullen) und ihre
 * Vergleichsform. Gelesen werden `-?\d+`, `-?\d+[.,]\d+`, `1.250,50`, `1,250.50` und Zahlwörter null bis zwölf; sonst `null`.
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
      kanonisch: kanonischeZahl(vorzeichen, einfach[1], bruch),
      vergleich: vergleichsform(einfach[1], bruch),
    };
  }
  const deutsch = /^([0-9]{1,3}(?:\.[0-9]{3})+),([0-9]+)$/.exec(k);
  if (deutsch) {
    const ganz = deutsch[1].replace(/\./g, "");
    return {
      kanonisch: kanonischeZahl(vorzeichen, ganz, deutsch[2]),
      vergleich: vergleichsform(ganz, deutsch[2]),
    };
  }
  const englisch = /^([0-9]{1,3}(?:,[0-9]{3})+)\.([0-9]+)$/.exec(k);
  if (englisch) {
    const ganz = englisch[1].replace(/,/g, "");
    return {
      kanonisch: kanonischeZahl(vorzeichen, ganz, englisch[2]),
      vergleich: vergleichsform(ganz, englisch[2]),
    };
  }
  const wort = vorzeichen === "" ? zahlwort(normalisiere(t)) : null;
  if (wort !== null)
    return { kanonisch: String(wort), vergleich: String(wort) };
  return null;
}

/** Ein Zahl-Token eines Textes (Codepunkte `von` bis `bis`): Ziffernfolgen, durch einzelne `.` oder `,` zwischen Ziffern verbunden. */
type ZahlToken = { von: number; bis: number; formen: string[] };

/** Die Zahl-Token eines normalisierten Textes mit ihren Vergleichsformen (leer, wenn die Folge keine Zahl ergibt, etwa `1.2.3`). */
function zahlTokens(c: readonly number[]): ZahlToken[] {
  const aus: ZahlToken[] = [];
  let i = 0;
  while (i < c.length) {
    if (!istZiffer(c[i])) {
      i += 1;
      continue;
    }
    const von = i;
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
    aus.push({ von, bis: i, formen: zahlKandidaten(gruppen, trenner) });
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

/**
 * Deckt die Spanne `start`..`ende` (Einheiten) des normalisierten `beitrag` im Kontext die Zahl mit der Vergleichsform `vergleich`?
 * Ein Zahl-Token, das die Spanne schneidet, wird in der Nachbarschaft gebildet (der Beleg `12` in `12,5 Liter` ergibt das Token
 * `12,5` und deckt 12 nicht); gedeckt ist, wenn eines dieser Token den Wert hat oder ein Zahlwort null bis zwölf als ganzes Wort in der
 * Spanne für ihn steht.
 */
function zahlDeckungImKontext(
  beitrag: string,
  start: number,
  ende: number,
  vergleich: string,
): boolean {
  const grenzen = nachbarschaft(beitrag, start, ende);
  if (grenzen === null) return false;
  const c = codepunkte(beitrag.slice(grenzen.von, grenzen.bis));
  const s = codepunkte(beitrag.slice(grenzen.von, start)).length;
  const e = s + codepunkte(beitrag.slice(start, ende)).length;
  const schneidet = (von: number, bis: number) => von < e && bis > s;
  if (
    zahlTokens(c).some(
      (t) => schneidet(t.von, t.bis) && t.formen.includes(vergleich),
    )
  ) {
    return true;
  }
  if (!/^[0-9]{1,2}$/.test(vergleich) || Number(vergleich) > 12) return false;
  const zahl = Number(vergleich);
  let i = 0;
  while (i < c.length) {
    if (!istWortzeichen(c[i])) {
      i += 1;
      continue;
    }
    let j = i;
    while (j < c.length && istWortzeichen(c[j])) j += 1;
    if (schneidet(i, j) && zahlwort(ausCodepunkten(c, i, j)) === zahl)
      return true;
    i = j;
  }
  return false;
}

/**
 * Nennt der Beleg `beleg` (normalisiert) im normalisierten `beitrag` die Zahl mit der Vergleichsform `vergleich`? An allen Fundstellen
 * (mindestens eine, höchstens `MAX_FUNDSTELLEN`) muss `zahlDeckungImKontext` gelten.
 */
export function zahlGedeckt(
  beitrag: string,
  beleg: string,
  vergleich: string,
): boolean {
  const stellen = fundstellen(beitrag, beleg, MAX_FUNDSTELLEN + 1);
  if (stellen.length === 0 || stellen.length > MAX_FUNDSTELLEN) return false;
  return stellen.every((s) =>
    zahlDeckungImKontext(beitrag, s, s + beleg.length, vergleich),
  );
}

/**
 * Steht `nadel` als ganze Wortfolge in `heuhaufen` (beide normalisiert)? Wie ein Teilstring, aber vor einem Treffer, der mit einem
 * Wortzeichen beginnt, und nach einem, der mit einem endet, steht kein Wortzeichen (`ja` steht nicht in `jahr`).
 */
export function enthaeltWortfolge(heuhaufen: string, nadel: string): boolean {
  return fundstellen(heuhaufen, nadel, 1).length > 0;
}

/** Der Codepunkt, der unmittelbar vor der Einheit `i` endet. */
function codepunktVor(s: string, i: number): number {
  const tief = s.charCodeAt(i - 1);
  if (tief >= 0xdc00 && tief <= 0xdfff && i >= 2) {
    const hoch = s.charCodeAt(i - 2);
    if (hoch >= 0xd800 && hoch <= 0xdbff) return s.codePointAt(i - 2)!;
  }
  return tief;
}
