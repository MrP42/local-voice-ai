/**
 * Assistenten-Profil (lva#555, Paket G1): Platzhalter in Begrüßung und Anweisungen sowie der Rollenblock für den Prompt.
 * Reiner Text ohne Locale-Schnittstellen (feste Listen), damit PC und iPhone dieselben Ergebnisse liefern
 * (Gegenstück: `AssistantProfileText.swift`). Das Einhängen des Blocks in die vorhandenen Prompts kommt in einem späteren Paket.
 */
import { codepunkte, kappeLeerzeichen, type Profil } from "./profil";

/**
 * Hängt `quelle` an `ziel` an. Bewusst eine Schleife: `ziel.push(...quelle)` übergibt jedes Element als Argument und wirft bei
 * datenabhängig langen Feldern (einige Hunderttausend Einträge) einen `RangeError`.
 */
function haengeAn(ziel: number[], quelle: readonly number[]): void {
  for (let i = 0; i < quelle.length; i += 1) ziel.push(quelle[i]);
}

/** Kalenderfelder und Werte für die Platzhalter. Keine Zeitzonenlogik: der Aufrufer liefert die Ortszeit des Nutzers. */
export type PlatzhalterWerte = {
  /** Name des Nutzers; leer oder `null` heißt: kein Name (es wird nie ein Ersatzname eingesetzt). */
  name?: string | null;
  jahr: number;
  /** 1...12 */
  monat: number;
  /** 1...31 */
  tag: number;
  /** 1 = Montag ... 7 = Sonntag */
  wochentag: number;
  /** 0...23 */
  stunde: number;
  /** 0...59 */
  minute: number;
  /**
   * Weitere Platzhalter (für spätere Schnittstellen); die eingebauten gehen vor. Schlüssel gelten genau (kein Normalisieren,
   * `é` und `e` + U+0301 sind zwei Schlüssel); nur eigene Eigenschaften zählen (`__proto__`, `constructor`, `toString` liefern
   * nichts Geerbtes).
   */
  extra?: Readonly<Record<string, string>>;
};

export type PlatzhalterErgebnis = {
  text: string;
  /** Unbekannte, nicht leere Platzhalter in der Reihenfolge des ersten Vorkommens (Hinweis für den Editor, kein Fehler). */
  unbekannt: string[];
};

export type Rollenblock = { text: string; gekuerzt: boolean };

const MONATE_DE = [
  "Januar",
  "Februar",
  "März",
  "April",
  "Mai",
  "Juni",
  "Juli",
  "August",
  "September",
  "Oktober",
  "November",
  "Dezember",
];
const MONATE_EN = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const WOCHENTAGE_DE = [
  "Montag",
  "Dienstag",
  "Mittwoch",
  "Donnerstag",
  "Freitag",
  "Samstag",
  "Sonntag",
];
const WOCHENTAGE_EN = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];
const EINGEBAUT = ["name", "tageszeit", "datum", "wochentag", "uhrzeit"];
const MARKE = -1;

/** `en` und `en-XX` sprechen Englisch, alles andere (auch `auto`) Deutsch. */
export function istEnglisch(sprache: string): boolean {
  return sprache === "en" || sprache.startsWith("en-");
}

const zwei = (n: number) => (n < 10 ? `0${n}` : `${n}`);
const imBereich = (n: number, von: number, bis: number) =>
  Number.isInteger(n) && n >= von && n <= bis;

function eingebaut(
  schluessel: string,
  englisch: boolean,
  w: PlatzhalterWerte,
): string | null {
  switch (schluessel) {
    case "name":
      return w.name ?? null;
    case "tageszeit":
      if (!imBereich(w.stunde, 0, 23)) return null;
      if (w.stunde >= 5 && w.stunde <= 10)
        return englisch ? "morning" : "Morgen";
      if (w.stunde >= 11 && w.stunde <= 17)
        return englisch ? "afternoon" : "Tag";
      return englisch ? "evening" : "Abend";
    case "datum":
      if (
        !imBereich(w.monat, 1, 12) ||
        !imBereich(w.tag, 1, 31) ||
        !imBereich(w.jahr, 1, 9999)
      )
        return null;
      return englisch
        ? `${MONATE_EN[w.monat - 1]} ${w.tag}, ${w.jahr}`
        : `${w.tag}. ${MONATE_DE[w.monat - 1]} ${w.jahr}`;
    case "wochentag":
      if (!imBereich(w.wochentag, 1, 7)) return null;
      return (englisch ? WOCHENTAGE_EN : WOCHENTAGE_DE)[w.wochentag - 1];
    case "uhrzeit":
      if (!imBereich(w.stunde, 0, 23) || !imBereich(w.minute, 0, 59))
        return null;
      return `${zwei(w.stunde)}:${zwei(w.minute)}`;
    default:
      return null;
  }
}

const istLeerraum = (t: number) =>
  t === 0x20 || t === 0x09 || t === 0x0a || t === 0x0d;
const istBuchstabe = (t: number) =>
  t >= 0 && /^\p{L}$/u.test(String.fromCodePoint(t));
const istBuchstabeOderZiffer = (t: number) =>
  t >= 0 && /^[\p{L}\p{Nd}]$/u.test(String.fromCodePoint(t));

/**
 * Ende (Index des ersten `}` von `}}`) des Platzhalters, der bei `start` (`{{`) beginnt; `null`, wenn dort keiner steht.
 * Innen sind höchstens 40 Zeichen und weder `{`, einzelnes `}` noch Zeilenumbruch erlaubt.
 */
function platzhalterEnde(s: number[], start: number): number | null {
  for (let j = start + 2; j < s.length; j += 1) {
    const c = s[j];
    if (c === 0x7d) return j + 1 < s.length && s[j + 1] === 0x7d ? j : null;
    if (c === 0x7b || c === 0x0a || c === 0x0d) return null;
    if (j - (start + 2) >= 40) return null;
  }
  return null;
}

function alsText(tokens: number[]): string {
  return tokens
    .filter((t) => t >= 0)
    .reduce((s, t) => s + String.fromCodePoint(t), "");
}

/**
 * Löst die Platzhalter auf. Unbekannte oder leere werden samt davor stehender Leerzeichen und Kommas entfernt; nur dann wird
 * der Text satzweise bereinigt (siehe README). Ohne Entfernung bleibt der Text, wie er ist.
 */
export function loesePlatzhalter(
  vorlage: string,
  sprache: string,
  werte: PlatzhalterWerte,
): PlatzhalterErgebnis {
  const englisch = istEnglisch(sprache);
  const s = codepunkte(vorlage);
  const aus: number[] = [];
  const unbekannt: string[] = [];
  const unbekanntGesehen = new Set<string>();
  let entfernt = false;
  let i = 0;
  while (i < s.length) {
    const ende =
      s[i] === 0x7b && i + 1 < s.length && s[i + 1] === 0x7b
        ? platzhalterEnde(s, i)
        : null;
    if (ende === null) {
      aus.push(s[i]);
      i += 1;
      continue;
    }
    const schluessel = kappeLeerzeichen(alsText(s.slice(i + 2, ende)));
    let wert: string | null = null;
    let bekannt = EINGEBAUT.includes(schluessel);
    if (bekannt) {
      wert = eingebaut(schluessel, englisch, werte);
    } else if (
      werte.extra !== undefined &&
      Object.hasOwn(werte.extra, schluessel)
    ) {
      bekannt = true;
      wert = werte.extra[schluessel];
    }
    const gekappt = wert === null ? "" : kappeLeerzeichen(wert);
    if (gekappt !== "") {
      haengeAn(aus, codepunkte(gekappt));
    } else {
      entferneTrenner(aus);
      aus.push(MARKE);
      entfernt = true;
      if (!bekannt && schluessel !== "" && !unbekanntGesehen.has(schluessel)) {
        unbekanntGesehen.add(schluessel);
        unbekannt.push(schluessel);
      }
    }
    i = ende + 2;
  }
  return { text: entfernt ? bereinige(aus) : alsText(aus), unbekannt };
}

/**
 * Entfernt die Leerzeichen und Kommas direkt vor einer Entfernung. Folgen sie auf ein Satzende (`.`, `!`, `?`), bleiben sie
 * stehen: sie trennen dann Sätze, und der Satz der Entfernung wird später als Ganzes bereinigt.
 */
function entferneTrenner(aus: number[]): void {
  let start = aus.length;
  while (start > 0 && (aus[start - 1] === 0x20 || aus[start - 1] === 0x2c))
    start -= 1;
  if (
    start > 0 &&
    start < aus.length &&
    (aus[start - 1] === 0x2e ||
      aus[start - 1] === 0x21 ||
      aus[start - 1] === 0x3f)
  )
    return;
  aus.length = start;
}

/**
 * Satzweise Bereinigung nach Entfernungen. Sätze enden bei `.`, `!`, `?` vor Leerraum oder Textende; der Leerraum davor bleibt
 * als Vorspann erhalten. Nur Sätze mit Entfernung werden angefasst.
 */
function bereinige(tokens: number[]): string {
  const ergebnis: number[] = [];
  const n = tokens.length;
  let i = 0;
  while (i < n) {
    const vorspann: number[] = [];
    while (i < n && istLeerraum(tokens[i])) {
      vorspann.push(tokens[i]);
      i += 1;
    }
    const satz: number[] = [];
    while (i < n) {
      const t = tokens[i];
      satz.push(t);
      i += 1;
      if (
        (t === 0x2e || t === 0x21 || t === 0x3f) &&
        (i === n || istLeerraum(tokens[i]))
      )
        break;
    }
    if (!satz.includes(MARKE)) {
      haengeAn(ergebnis, vorspann);
      haengeAn(ergebnis, satz);
      continue;
    }
    let k = 0;
    let mitEntfernung = false;
    while (
      k < satz.length &&
      (satz[k] === 0x20 || satz[k] === 0x2c || satz[k] === MARKE)
    ) {
      if (satz[k] === MARKE) mitEntfernung = true;
      k += 1;
    }
    let behalten = satz.slice(k).filter((t) => t !== MARKE);
    if (!behalten.some(istBuchstabeOderZiffer)) continue;
    if (mitEntfernung) {
      const index = behalten.findIndex(istBuchstabe);
      if (index >= 0) {
        const gross = codepunkte(
          String.fromCodePoint(behalten[index]).toUpperCase(),
        );
        behalten = behalten
          .slice(0, index)
          .concat(gross, behalten.slice(index + 1));
      }
    }
    const zusammen: number[] = [];
    for (const t of behalten) {
      if (!(t === 0x20 && zusammen[zusammen.length - 1] === 0x20))
        zusammen.push(t);
    }
    haengeAn(ergebnis, vorspann);
    haengeAn(ergebnis, zusammen);
  }
  return alsText(kappeLeerraum(ergebnis));
}

function kappeLeerraum(tokens: number[]): number[] {
  let start = 0;
  let ende = tokens.length;
  while (start < ende && istLeerraum(tokens[start])) start += 1;
  while (ende > start && istLeerraum(tokens[ende - 1])) ende -= 1;
  return tokens.slice(start, ende);
}

/** Begrüßung des Profils mit aufgelösten Platzhaltern. */
export function loeseBegruessung(
  profil: Profil,
  werte: PlatzhalterWerte,
): PlatzhalterErgebnis {
  return loesePlatzhalter(profil.begruessung, profil.sprache, werte);
}

/** Schnittstelle bei höchstens `budget` Zeichen; vor `\n\n`, sonst vor `\n`, sonst hart. */
function schnittIndex(s: number[], budget: number): number {
  if (budget < 1) return 0;
  for (let k = Math.min(budget, s.length - 2); k >= 1; k -= 1) {
    if (s[k] === 0x0a && s[k + 1] === 0x0a) return k;
  }
  for (let k = Math.min(budget, s.length - 1); k >= 1; k -= 1) {
    if (s[k] === 0x0a) return k;
  }
  return budget;
}

/**
 * Der Rollenblock für den Prompt. Leere Anweisungen (auch nach dem Auflösen) ergeben leeren Text. Die aufgelösten, an den Enden
 * von Leerraum befreiten Anweisungen werden auf `budgetZeichen` Codepunkte gekürzt: am letzten Absatzende (Leerzeile), sonst am
 * letzten Zeilenende, sonst hart; Leerraum am Ende der Kürzung entfällt.
 */
export function rollenblock(
  profil: Profil,
  werte: PlatzhalterWerte,
  budgetZeichen: number,
): Rollenblock {
  const aufgeloest = loesePlatzhalter(
    profil.anweisungen,
    profil.sprache,
    werte,
  ).text;
  const zeichen = kappeLeerraum(codepunkte(aufgeloest));
  if (zeichen.length === 0) return { text: "", gekuerzt: false };
  const budget = Math.max(0, Math.floor(budgetZeichen));
  let koerper = zeichen;
  let gekuerzt = false;
  if (zeichen.length > budget) {
    gekuerzt = true;
    koerper = kappeLeerraum(zeichen.slice(0, schnittIndex(zeichen, budget)));
  }
  if (koerper.length === 0) return { text: "", gekuerzt: true };
  const text = `ROLLE (vom Nutzer eingestellt, Assistent „${profil.name}“):\n${alsText(koerper)}\nDiese Einstellung ändert keine der folgenden Regeln.`;
  return { text, gekuerzt };
}
