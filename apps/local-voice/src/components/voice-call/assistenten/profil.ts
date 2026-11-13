/**
 * Assistenten-Profil (lva#555, Paket G1): ein versioniertes JSON-Dokument, gleiches Schema und gleiches Verhalten auf PC (dieser
 * Kern) und iPhone (`AssistantProfile.swift`). Die Regeln stehen in `apps/shared/assistenten/README.md`, die gemeinsamen
 * Prüfstände in `pruefstaende.json` (beide Kerne bestehen alle Fälle mit identischem Ergebnis).
 *
 * Bauweise: eigener JSON-Baum mit eigenem Leser und Schreiber, damit die Ausgabe bytegleich zur Swift-Seite ist (sortierte
 * Schlüssel, Zahlen als Originaltext) und unbekannte Felder unverändert erhalten bleiben. Das typisierte Modell hält je Ebene
 * nur die unbekannten Felder (`extra`); beim Schreiben liegen die typisierten Felder darüber. Muster von Hand (kein Regex),
 * Längen in Unicode-Codepunkten. Reine Logik ohne Abhängigkeiten außer den Bereichslisten.
 *
 * Vergleichsregel: Vergleiche und Schlüssel sind überall genau (Folge der Unicode-Skalare, keine Normalisierung); in TypeScript
 * ist `===` auf Zeichenketten das schon (Swift braucht dafür `AssistantExact`). Aufrufe mit `push(...feld)` über datenabhängig
 * lange Felder sind verboten (`RangeError`); hier und in `text.ts` laufen solche Anhänge über Schleifen oder `concat`.
 */
import { ALL_AREAS } from "../bereiche";

// ---------------------------------------------------------------- JSON-Baum

export type JsonMitglied = { schluessel: string; wert: JsonWert };
/** JSON-Wert; Zahlen bleiben als Originaltext (`roh`), so gehen fremde Zahlen unverändert durch. */
export type JsonWert =
  | { art: "null" }
  | { art: "bool"; wert: boolean }
  | { art: "zahl"; roh: string }
  | { art: "text"; wert: string }
  | { art: "liste"; wert: JsonWert[] }
  | { art: "objekt"; wert: JsonMitglied[] };

const JSON_MAX_TIEFE = 64;

/** Vergleich nach Codepunkten (gleich der UTF-8-Byte-Reihenfolge), nicht nach UTF-16-Einheiten. */
export function vergleicheCodepunkte(a: string, b: string): number {
  const ia = a[Symbol.iterator]();
  const ib = b[Symbol.iterator]();
  for (;;) {
    const x = ia.next();
    const y = ib.next();
    if (x.done || y.done) return x.done === y.done ? 0 : x.done ? -1 : 1;
    const cx = x.value.codePointAt(0)!;
    const cy = y.value.codePointAt(0)!;
    if (cx !== cy) return cx < cy ? -1 : 1;
  }
}

export function mitglied(
  objekt: JsonMitglied[],
  schluessel: string,
): JsonWert | undefined {
  return objekt.find((m) => m.schluessel === schluessel)?.wert;
}

export function setzeMitglied(
  objekt: JsonMitglied[],
  schluessel: string,
  wert: JsonWert,
): void {
  const vorhanden = objekt.find((m) => m.schluessel === schluessel);
  if (vorhanden) vorhanden.wert = wert;
  else objekt.push({ schluessel, wert });
}

/** Tiefe, unabhängige Kopie eines JSON-Werts (ein Profil teilt keine Knoten mit seiner Quelle, auch nicht mit eingefrorenen). */
function klone(wert: JsonWert): JsonWert {
  switch (wert.art) {
    case "liste":
      return { art: "liste", wert: wert.wert.map(klone) };
    case "objekt":
      return {
        art: "objekt",
        wert: wert.wert.map((m) => ({
          schluessel: m.schluessel,
          wert: klone(m.wert),
        })),
      };
    default:
      return { ...wert };
  }
}

/** Flache Kopie der Mitglieder (die Werte bleiben geteilt; für den kurzlebigen Baum von `profilBaum`). */
const kopiereMitglieder = (objekt: JsonMitglied[]): JsonMitglied[] =>
  objekt.map((m) => ({ ...m }));

/**
 * `true`, wenn der Leser den Wert in dieser Tiefe wieder läse: Tiefengrenze (Wurzel 0, wie `JsonLeser`), Zahlen nach
 * JSON-Grammatik, keine doppelten Schlüssel, keine einzelnen Surrogate in Zeichenketten und Schlüsseln.
 */
function lesbar(wert: JsonWert, tiefe: number): boolean {
  if (tiefe > JSON_MAX_TIEFE) return false;
  switch (wert.art) {
    case "null":
    case "bool":
      return true;
    case "zahl": {
      const gelesen = parseJson(wert.roh);
      return (
        gelesen !== null && gelesen.art === "zahl" && gelesen.roh === wert.roh
      );
    }
    case "text":
      return !hatEinzelnesSurrogat(wert.wert);
    case "liste":
      return wert.wert.every((e) => lesbar(e, tiefe + 1));
    case "objekt": {
      const gesehen = new Set<string>();
      for (const m of wert.wert) {
        if (gesehen.has(m.schluessel) || hatEinzelnesSurrogat(m.schluessel))
          return false;
        gesehen.add(m.schluessel);
        if (!lesbar(m.wert, tiefe + 1)) return false;
      }
      return true;
    }
  }
}

/** Ein einzelnes Surrogat (ohne Partner) in der Zeichenkette: sie ließe sich nicht als UTF-8 schreiben und wieder lesen. */
function hatEinzelnesSurrogat(s: string): boolean {
  for (let i = 0; i < s.length; i += 1) {
    const c = s.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff) {
      const n = i + 1 < s.length ? s.charCodeAt(i + 1) : -1;
      if (n < 0xdc00 || n > 0xdfff) return true;
      i += 1;
    } else if (c >= 0xdc00 && c <= 0xdfff) {
      return true;
    }
  }
  return false;
}

class JsonLeser {
  private pos = 0;
  private readonly s: string;
  constructor(s: string) {
    this.s = s;
  }

  dokument(): JsonWert | null {
    this.leerraum();
    const wert = this.wert(0);
    if (wert === null) return null;
    this.leerraum();
    return this.pos === this.s.length ? wert : null;
  }

  private c(): number {
    return this.pos < this.s.length ? this.s.charCodeAt(this.pos) : -1;
  }

  private leerraum(): void {
    for (;;) {
      const c = this.c();
      if (c === 0x20 || c === 0x09 || c === 0x0a || c === 0x0d) this.pos += 1;
      else return;
    }
  }

  private wort(w: string): boolean {
    if (!this.s.startsWith(w, this.pos)) return false;
    this.pos += w.length;
    return true;
  }

  private wert(tiefe: number): JsonWert | null {
    if (tiefe > JSON_MAX_TIEFE) return null;
    switch (this.c()) {
      case 0x7b:
        return this.objekt(tiefe);
      case 0x5b:
        return this.liste(tiefe);
      case 0x22: {
        const t = this.text();
        return t === null ? null : { art: "text", wert: t };
      }
      case 0x74:
        return this.wort("true") ? { art: "bool", wert: true } : null;
      case 0x66:
        return this.wort("false") ? { art: "bool", wert: false } : null;
      case 0x6e:
        return this.wort("null") ? { art: "null" } : null;
      default:
        return this.zahl();
    }
  }

  private objekt(tiefe: number): JsonWert | null {
    this.pos += 1;
    const mitglieder: JsonMitglied[] = [];
    this.leerraum();
    if (this.c() === 0x7d) {
      this.pos += 1;
      return { art: "objekt", wert: mitglieder };
    }
    for (;;) {
      this.leerraum();
      if (this.c() !== 0x22) return null;
      const schluessel = this.text();
      if (schluessel === null) return null;
      this.leerraum();
      if (this.c() !== 0x3a) return null;
      this.pos += 1;
      this.leerraum();
      const wert = this.wert(tiefe + 1);
      if (wert === null) return null;
      setzeMitglied(mitglieder, schluessel, wert);
      this.leerraum();
      const c = this.c();
      this.pos += 1;
      if (c === 0x2c) continue;
      if (c === 0x7d) return { art: "objekt", wert: mitglieder };
      return null;
    }
  }

  private liste(tiefe: number): JsonWert | null {
    this.pos += 1;
    const eintraege: JsonWert[] = [];
    this.leerraum();
    if (this.c() === 0x5d) {
      this.pos += 1;
      return { art: "liste", wert: eintraege };
    }
    for (;;) {
      this.leerraum();
      const wert = this.wert(tiefe + 1);
      if (wert === null) return null;
      eintraege.push(wert);
      this.leerraum();
      const c = this.c();
      this.pos += 1;
      if (c === 0x2c) continue;
      if (c === 0x5d) return { art: "liste", wert: eintraege };
      return null;
    }
  }

  private hex4(): number | null {
    if (this.pos + 4 > this.s.length) return null;
    const teil = this.s.slice(this.pos, this.pos + 4);
    if (!/^[0-9a-fA-F]{4}$/.test(teil)) return null;
    this.pos += 4;
    return parseInt(teil, 16);
  }

  private text(): string | null {
    this.pos += 1;
    let aus = "";
    for (;;) {
      if (this.pos >= this.s.length) return null;
      const c = this.s.charCodeAt(this.pos);
      if (c === 0x22) {
        this.pos += 1;
        return aus;
      }
      if (c < 0x20) return null;
      if (c !== 0x5c) {
        if (c >= 0xd800 && c <= 0xdbff) {
          const n =
            this.pos + 1 < this.s.length ? this.s.charCodeAt(this.pos + 1) : -1;
          if (n < 0xdc00 || n > 0xdfff) return null;
          aus += this.s.slice(this.pos, this.pos + 2);
          this.pos += 2;
          continue;
        }
        if (c >= 0xdc00 && c <= 0xdfff) return null;
        aus += this.s[this.pos];
        this.pos += 1;
        continue;
      }
      this.pos += 1;
      if (this.pos >= this.s.length) return null;
      const e = this.s[this.pos];
      this.pos += 1;
      switch (e) {
        case '"':
          aus += '"';
          break;
        case "\\":
          aus += "\\";
          break;
        case "/":
          aus += "/";
          break;
        case "b":
          aus += "\b";
          break;
        case "f":
          aus += "\f";
          break;
        case "n":
          aus += "\n";
          break;
        case "r":
          aus += "\r";
          break;
        case "t":
          aus += "\t";
          break;
        case "u": {
          const code = this.hex4();
          if (code === null || (code >= 0xdc00 && code <= 0xdfff)) return null;
          if (code >= 0xd800 && code <= 0xdbff) {
            if (!this.s.startsWith("\\u", this.pos)) return null;
            this.pos += 2;
            const tief = this.hex4();
            if (tief === null || tief < 0xdc00 || tief > 0xdfff) return null;
            aus += String.fromCharCode(code, tief);
          } else {
            aus += String.fromCharCode(code);
          }
          break;
        }
        default:
          return null;
      }
    }
  }

  private ziffern(): boolean {
    const start = this.pos;
    while (this.c() >= 0x30 && this.c() <= 0x39) this.pos += 1;
    return this.pos > start;
  }

  private zahl(): JsonWert | null {
    const start = this.pos;
    if (this.c() === 0x2d) this.pos += 1;
    if (this.c() === 0x30) this.pos += 1;
    else if (this.c() >= 0x31 && this.c() <= 0x39) this.ziffern();
    else return null;
    if (this.c() === 0x2e) {
      this.pos += 1;
      if (!this.ziffern()) return null;
    }
    if (this.c() === 0x65 || this.c() === 0x45) {
      this.pos += 1;
      if (this.c() === 0x2b || this.c() === 0x2d) this.pos += 1;
      if (!this.ziffern()) return null;
    }
    return { art: "zahl", roh: this.s.slice(start, this.pos) };
  }
}

/** Liest strenges JSON (RFC 8259, ein Wert, nichts danach); `null` bei Fehler. Doppelte Schlüssel: der letzte gilt. */
export function parseJson(text: string): JsonWert | null {
  return new JsonLeser(text).dokument();
}

function schreibeText(s: string): string {
  let aus = '"';
  for (let i = 0; i < s.length; i += 1) {
    const c = s.charCodeAt(i);
    if (c === 0x22) aus += '\\"';
    else if (c === 0x5c) aus += "\\\\";
    else if (c === 0x0a) aus += "\\n";
    else if (c === 0x0d) aus += "\\r";
    else if (c === 0x09) aus += "\\t";
    else if (c === 0x08) aus += "\\b";
    else if (c === 0x0c) aus += "\\f";
    else if (c < 0x20) aus += `\\u${c.toString(16).padStart(4, "0")}`;
    else aus += s[i];
  }
  return `${aus}"`;
}

function schreibe(wert: JsonWert, ebene: number): string {
  const einzug = (n: number) => "  ".repeat(n);
  switch (wert.art) {
    case "null":
      return "null";
    case "bool":
      return wert.wert ? "true" : "false";
    case "zahl":
      return wert.roh;
    case "text":
      return schreibeText(wert.wert);
    case "liste":
      if (wert.wert.length === 0) return "[]";
      return `[\n${wert.wert
        .map((e) => einzug(ebene + 1) + schreibe(e, ebene + 1))
        .join(",\n")}\n${einzug(ebene)}]`;
    case "objekt": {
      if (wert.wert.length === 0) return "{}";
      const sortiert = wert.wert
        .slice()
        .sort((a, b) => vergleicheCodepunkte(a.schluessel, b.schluessel));
      return `{\n${sortiert
        .map(
          (m) =>
            `${einzug(ebene + 1)}${schreibeText(m.schluessel)}: ${schreibe(m.wert, ebene + 1)}`,
        )
        .join(",\n")}\n${einzug(ebene)}}`;
    }
  }
}

/** Kanonische Ausgabe: Schlüssel nach Codepunkten sortiert, 2 Leerzeichen Einzug, `\n` am Ende. */
export function kodiereJson(wert: JsonWert): string {
  return `${schreibe(wert, 0)}\n`;
}

export const jsonText = (wert: string): JsonWert => ({ art: "text", wert });
export const jsonBool = (wert: boolean): JsonWert => ({ art: "bool", wert });
export const jsonGanzzahl = (wert: number): JsonWert => ({
  art: "zahl",
  roh: String(wert),
});
const jsonListe = (wert: JsonWert[]): JsonWert => ({ art: "liste", wert });
const jsonObjekt = (wert: JsonMitglied[]): JsonWert => ({
  art: "objekt",
  wert,
});
const jsonTexte = (werte: string[]): JsonWert => jsonListe(werte.map(jsonText));

// ---------------------------------------------------------------- Fehler

export const FEHLER_CODES = [
  "fehlt",
  "typ",
  "laenge",
  "muster",
  "wert",
  "zu_viele",
  "doppelt",
  "schema_unbekannt",
  "schema_neuer",
  "json",
] as const;
export type FehlerCode = (typeof FEHLER_CODES)[number];
/** Ein Befund der Prüfung: Pfad (`name`, `wissen.kontexte[3].art`) und Code. Es wird nie stillschweigend repariert. */
export type ProfilFehler = { pfad: string; code: FehlerCode };

/** Sortiert nach Pfad, dann Code (Codepunkte) und entfernt Wiederholungen. */
export function ordneFehler(fehler: ProfilFehler[]): ProfilFehler[] {
  const gesehen = new Set<string>();
  const einmalig = fehler.filter((f) => {
    const schluessel = `${f.pfad}\u0000${f.code}`;
    if (gesehen.has(schluessel)) return false;
    gesehen.add(schluessel);
    return true;
  });
  return einmalig.sort(
    (a, b) =>
      vergleicheCodepunkte(a.pfad, b.pfad) ||
      vergleicheCodepunkte(a.code, b.code),
  );
}

// ---------------------------------------------------------------- Modell

export const PROFIL_SCHEMA = "lva.assistent/1";
/** Bekannte Bereichsschlüssel des Profils. */
export const BEKANNTE_BEREICHE = [
  "buecher",
  "buchnotizen",
  "ki_dokumente",
  "besprechungen",
  "sprachnotizen",
  "ablage",
  "gespraeche",
  "kalender",
  "vorlesen",
  "notizbuch",
] as const;

export const STILE = ["natuerlich", "woertlich"] as const;
export const SPRECHPAUSEN = ["kurz", "normal", "lang"] as const;
export const MODI = ["alles", "zuerst", "nur"] as const;
export const KONTEXT_ARTEN = [
  "buch",
  "besprechung",
  "sprachnotiz",
  "datei",
  "ordner",
  "projekt",
  "abschnitt",
] as const;
export const FELD_TYPEN = ["text", "zahl", "datum", "auswahl"] as const;

export type Verhalten = {
  stil: (typeof STILE)[number];
  sprechpause: (typeof SPRECHPAUSEN)[number];
  unterbrechen: boolean;
  /** 0 (aus) oder 5...120 Sekunden Stille bis zur Nachfrage. */
  nachfrageStilleS: number;
  extra: JsonMitglied[];
};
export type Kontext = {
  art: (typeof KONTEXT_ARTEN)[number];
  id: string;
  titel: string;
  extra: JsonMitglied[];
};
export type Wissen = {
  alleBereiche: boolean;
  bereiche: string[];
  modus: (typeof MODI)[number];
  kontexte: Kontext[];
  extra: JsonMitglied[];
};
export type AuswertungFeld = {
  name: string;
  beschreibung: string;
  typ: (typeof FELD_TYPEN)[number];
  optionen: string[];
  extra: JsonMitglied[];
};
export type Auswertung = {
  zusammenfassung: boolean;
  aufgaben: boolean;
  felder: AuswertungFeld[];
  extra: JsonMitglied[];
};
export type Profil = {
  id: string;
  name: string;
  beschreibung: string;
  anweisungen: string;
  begruessung: string;
  sprache: string;
  stimme: string | null;
  verhalten: Verhalten;
  wissen: Wissen;
  schnittstellen: string[];
  auswertung: Auswertung;
  /** Unbekannte Felder der obersten Ebene. */
  extra: JsonMitglied[];
};

type ProfilGelesen = { ok: true; profil: Profil };
type ProfilAbgewiesen = { ok: false; fehler: ProfilFehler[] };
export type ProfilErgebnis = ProfilGelesen | ProfilAbgewiesen;

export function neuesVerhalten(): Verhalten {
  return {
    stil: "natuerlich",
    sprechpause: "normal",
    unterbrechen: true,
    nachfrageStilleS: 0,
    extra: [],
  };
}
export function neuesWissen(): Wissen {
  return {
    alleBereiche: false,
    bereiche: [],
    modus: "alles",
    kontexte: [],
    extra: [],
  };
}
export function neueAuswertung(): Auswertung {
  return { zusammenfassung: false, aufgaben: false, felder: [], extra: [] };
}
export function neuesProfil(id: string, name: string): Profil {
  return {
    id,
    name,
    beschreibung: "",
    anweisungen: "",
    begruessung: "",
    sprache: "de",
    stimme: null,
    verhalten: neuesVerhalten(),
    wissen: neuesWissen(),
    schnittstellen: [],
    auswertung: neueAuswertung(),
    extra: [],
  };
}

// ---------------------------------------------------------------- Muster

const klein = (c: number) => c >= 0x61 && c <= 0x7a;
const gross = (c: number) => c >= 0x41 && c <= 0x5a;
const ziffer = (c: number) => c >= 0x30 && c <= 0x39;

export function codepunkte(s: string): number[] {
  return Array.from(s, (z) => z.codePointAt(0)!);
}
export function zaehle(s: string): number {
  return codepunkte(s).length;
}

function passt(
  s: string,
  maxLaenge: number,
  erstes: (c: number) => boolean,
  rest: (c: number) => boolean,
): boolean {
  const c = codepunkte(s);
  return (
    c.length >= 1 &&
    c.length <= maxLaenge &&
    erstes(c[0]) &&
    c.slice(1).every(rest)
  );
}

/** `^[a-z0-9][a-z0-9-]{0,63}$` */
export function istProfilId(s: string): boolean {
  return passt(
    s,
    64,
    (c) => klein(c) || ziffer(c),
    (c) => klein(c) || ziffer(c) || c === 0x2d,
  );
}
/**
 * Gerätenamen von Windows, die als Dateiname nicht taugen: `con`, `prn`, `aux`, `nul`, `com0`...`com9`, `lpt0`...`lpt9`.
 * Gilt für Ids, die das Id-Muster schon erfüllen (nur ASCII).
 */
export function istGeraetename(s: string): boolean {
  return /^(con|prn|aux|nul|(com|lpt)[0-9])$/.test(s);
}
/** `^[a-z][a-z0-9_]{0,39}$` (Bereichsschlüssel und Feldnamen) */
export function istSchluessel(s: string): boolean {
  return passt(s, 40, klein, (c) => klein(c) || ziffer(c) || c === 0x5f);
}
/** `^[a-z0-9][a-z0-9._-]{0,63}$` */
export function istSchnittstelle(s: string): boolean {
  return passt(
    s,
    64,
    (c) => klein(c) || ziffer(c),
    (c) => klein(c) || ziffer(c) || c === 0x2e || c === 0x5f || c === 0x2d,
  );
}
/** `"auto"` oder `^[a-z]{2}(-[A-Z]{2})?$` */
export function istSprache(s: string): boolean {
  if (s === "auto") return true;
  const c = codepunkte(s);
  if (c.length === 2) return klein(c[0]) && klein(c[1]);
  return (
    c.length === 5 &&
    klein(c[0]) &&
    klein(c[1]) &&
    c[2] === 0x2d &&
    gross(c[3]) &&
    gross(c[4])
  );
}
/** Steuerzeichen: U+0000...U+001F, U+007F...U+009F, U+2028, U+2029. */
function istSteuerzeichen(c: number): boolean {
  return c < 0x20 || (c >= 0x7f && c <= 0x9f) || c === 0x2028 || c === 0x2029;
}
/** Entfernt nur U+0020 an beiden Enden. */
export function kappeLeerzeichen(s: string): string {
  let start = 0;
  let ende = s.length;
  while (start < ende && s[start] === " ") start += 1;
  while (ende > start && s[ende - 1] === " ") ende -= 1;
  return s.slice(start, ende);
}

function schemaCode(s: string): FehlerCode {
  const vorsatz = "lva.assistent/";
  if (!s.startsWith(vorsatz)) return "schema_unbekannt";
  const rest = s.slice(vorsatz.length);
  if (!/^[1-9][0-9]*$/.test(rest)) return "schema_unbekannt";
  return rest === "1" ? "schema_unbekannt" : "schema_neuer";
}

/** Ganzzahl-Schreibweise `-?(0|[1-9][0-9]*)` bis 15 Ziffern; Bruch oder Exponent sind keine Ganzzahl. */
function leseGanzzahl(
  wert: JsonWert,
): "keine" | "ausserhalb" | { zahl: number } {
  if (wert.art !== "zahl") return "keine";
  const ziffern = wert.roh.startsWith("-") ? wert.roh.slice(1) : wert.roh;
  if (!/^(0|[1-9][0-9]*)$/.test(ziffern)) return "keine";
  if (ziffern.length > 15) return "ausserhalb";
  return { zahl: Number(wert.roh) };
}

// ---------------------------------------------------------------- Prüfung

class Pruefer {
  fehler: ProfilFehler[] = [];

  melde(pfad: string, code: FehlerCode): void {
    this.fehler.push({ pfad, code });
  }

  text(
    o: JsonMitglied[],
    schluessel: string,
    pfad: string,
    pflicht = false,
  ): string | undefined {
    const w = mitglied(o, schluessel);
    if (w === undefined) {
      if (pflicht) this.melde(pfad, "fehlt");
      return undefined;
    }
    if (w.art !== "text") {
      this.melde(pfad, "typ");
      return undefined;
    }
    this.wohlgeformt(w.wert, pfad);
    return w.wert;
  }

  /** Einzelne Surrogate in einer Zeichenkette: `json` am Pfad des Feldes. */
  wohlgeformt(s: string, pfad: string): void {
    if (hatEinzelnesSurrogat(s)) this.melde(pfad, "json");
  }

  /** Doppelte Schlüssel in einem Objekt: `json` am Pfad des Objekts. */
  pruefeSchluessel(o: JsonMitglied[], pfad: string): void {
    const gesehen = new Set<string>();
    for (const m of o) {
      if (gesehen.has(m.schluessel)) {
        this.melde(pfad, "json");
        return;
      }
      gesehen.add(m.schluessel);
    }
  }

  /**
   * Die unbekannten Mitglieder eines Objekts als Kopie; jedes wird vollständig geprüft (Befund `json` am Pfad des Feldes):
   * Tiefengrenze des Lesers, Zahlen nach JSON-Grammatik, keine doppelten Schlüssel, keine einzelnen Surrogate.
   * `tiefe` ist die Tiefe der Werte dieses Objekts (Wurzel: 1).
   */
  unbekannte(
    o: JsonMitglied[],
    bekannt: readonly string[],
    pfad: string,
    tiefe: number,
  ): JsonMitglied[] {
    const rest: JsonMitglied[] = [];
    for (const m of o) {
      if (bekannt.includes(m.schluessel)) continue;
      if (lesbar(m.wert, tiefe) && !hatEinzelnesSurrogat(m.schluessel)) {
        rest.push({ schluessel: m.schluessel, wert: klone(m.wert) });
        continue;
      }
      this.melde(
        pfad === "" ? m.schluessel : `${pfad}.${m.schluessel}`,
        "json",
      );
      rest.push({ ...m });
    }
    return rest;
  }

  flagge(o: JsonMitglied[], schluessel: string, pfad: string) {
    const w = mitglied(o, schluessel);
    if (w === undefined) return undefined;
    if (w.art !== "bool") {
      this.melde(pfad, "typ");
      return undefined;
    }
    return w.wert;
  }

  liste(o: JsonMitglied[], schluessel: string, pfad: string, pflicht = false) {
    const w = mitglied(o, schluessel);
    if (w === undefined) {
      if (pflicht) this.melde(pfad, "fehlt");
      return undefined;
    }
    if (w.art !== "liste") {
      this.melde(pfad, "typ");
      return undefined;
    }
    return w.wert;
  }

  objekt(o: JsonMitglied[], schluessel: string, pfad: string) {
    const w = mitglied(o, schluessel);
    if (w === undefined) return undefined;
    if (w.art !== "objekt") {
      this.melde(pfad, "typ");
      return undefined;
    }
    return w.wert;
  }

  /** Text mit Längengrenzen in Codepunkten; bei Verstoß `laenge`. */
  begrenzt(s: string | undefined, pfad: string, min: number, max: number) {
    if (s === undefined) return undefined;
    const n = zaehle(s);
    if (n < min || n > max) this.melde(pfad, "laenge");
    return s;
  }

  texte(eintraege: JsonWert[], pfad: string, min: number, max: number) {
    const aus: string[] = [];
    eintraege.forEach((e, i) => {
      if (e.art !== "text") {
        this.melde(`${pfad}[${i}]`, "typ");
        return;
      }
      const n = zaehle(e.wert);
      if (n < min || n > max) this.melde(`${pfad}[${i}]`, "laenge");
      this.wohlgeformt(e.wert, `${pfad}[${i}]`);
      aus.push(e.wert);
    });
    return aus;
  }

  profil(wurzel: JsonMitglied[]): Profil | null {
    const schema = mitglied(wurzel, "schema");
    if (schema === undefined) this.melde("schema", "fehlt");
    else if (schema.art !== "text") this.melde("schema", "typ");
    else if (schema.wert !== PROFIL_SCHEMA) {
      this.fehler = [{ pfad: "schema", code: schemaCode(schema.wert) }];
      return null;
    }
    this.pruefeSchluessel(wurzel, "");

    const id = this.text(wurzel, "id", "id", true);
    if (id !== undefined) {
      if (!istProfilId(id)) this.melde("id", "muster");
      else if (istGeraetename(id)) this.melde("id", "wert");
    }
    const rohName = this.text(wurzel, "name", "name", true);
    const name = rohName === undefined ? undefined : kappeLeerzeichen(rohName);
    if (name !== undefined) {
      const n = zaehle(name);
      if (n < 1 || n > 60) this.melde("name", "laenge");
      if (codepunkte(name).some(istSteuerzeichen)) this.melde("name", "muster");
    }
    const profil = neuesProfil(id ?? "", name ?? "");
    profil.beschreibung =
      this.begrenzt(
        this.text(wurzel, "beschreibung", "beschreibung"),
        "beschreibung",
        0,
        200,
      ) ?? "";
    profil.anweisungen =
      this.begrenzt(
        this.text(wurzel, "anweisungen", "anweisungen"),
        "anweisungen",
        0,
        8000,
      ) ?? "";
    profil.begruessung =
      this.begrenzt(
        this.text(wurzel, "begruessung", "begruessung"),
        "begruessung",
        0,
        300,
      ) ?? "";

    const sprache = this.text(wurzel, "sprache", "sprache");
    if (sprache !== undefined) {
      if (!istSprache(sprache)) this.melde("sprache", "muster");
      profil.sprache = sprache;
    }
    const stimme = mitglied(wurzel, "stimme");
    if (stimme !== undefined && stimme.art !== "null") {
      if (stimme.art === "text") {
        this.wohlgeformt(stimme.wert, "stimme");
        profil.stimme = this.begrenzt(stimme.wert, "stimme", 1, 120) ?? null;
      } else this.melde("stimme", "typ");
    }

    const verhalten = this.objekt(wurzel, "verhalten", "verhalten");
    if (verhalten) profil.verhalten = this.verhalten(verhalten);
    const wissen = this.objekt(wurzel, "wissen", "wissen");
    if (wissen) profil.wissen = this.wissen(wissen);
    const schnittstellen = this.liste(
      wurzel,
      "schnittstellen",
      "schnittstellen",
    );
    if (schnittstellen)
      profil.schnittstellen = this.schnittstellen(schnittstellen);
    const auswertung = this.objekt(wurzel, "auswertung", "auswertung");
    if (auswertung) profil.auswertung = this.auswertung(auswertung);

    profil.extra = this.unbekannte(
      wurzel,
      [
        "schema",
        "id",
        "name",
        "beschreibung",
        "anweisungen",
        "begruessung",
        "sprache",
        "stimme",
        "verhalten",
        "wissen",
        "schnittstellen",
        "auswertung",
      ],
      "",
      1,
    );
    return profil;
  }

  verhalten(o: JsonMitglied[]): Verhalten {
    const v = neuesVerhalten();
    this.pruefeSchluessel(o, "verhalten");
    const stil = this.text(o, "stil", "verhalten.stil");
    if (stil !== undefined) {
      const treffer = STILE.find((s) => s === stil);
      if (treffer) v.stil = treffer;
      else this.melde("verhalten.stil", "wert");
    }
    const pause = this.text(o, "sprechpause", "verhalten.sprechpause");
    if (pause !== undefined) {
      const treffer = SPRECHPAUSEN.find((s) => s === pause);
      if (treffer) v.sprechpause = treffer;
      else this.melde("verhalten.sprechpause", "wert");
    }
    const unterbrechen = this.flagge(
      o,
      "unterbrechen",
      "verhalten.unterbrechen",
    );
    if (unterbrechen !== undefined) v.unterbrechen = unterbrechen;
    const stille = mitglied(o, "nachfrage_stille_s");
    if (stille !== undefined) {
      const pfad = "verhalten.nachfrage_stille_s";
      const n = leseGanzzahl(stille);
      if (n === "keine") this.melde(pfad, "typ");
      else if (n === "ausserhalb") this.melde(pfad, "wert");
      else if (n.zahl === 0 || (n.zahl >= 5 && n.zahl <= 120))
        v.nachfrageStilleS = n.zahl;
      else this.melde(pfad, "wert");
    }
    v.extra = this.unbekannte(
      o,
      ["stil", "sprechpause", "unterbrechen", "nachfrage_stille_s"],
      "verhalten",
      2,
    );
    return v;
  }

  wissen(o: JsonMitglied[]): Wissen {
    const k = neuesWissen();
    this.pruefeSchluessel(o, "wissen");
    const alle = this.flagge(o, "alle_bereiche", "wissen.alle_bereiche");
    if (alle !== undefined) k.alleBereiche = alle;
    const bereiche = this.liste(o, "bereiche", "wissen.bereiche");
    if (bereiche) {
      const gesehen = new Set<string>();
      bereiche.forEach((b, i) => {
        const pfad = `wissen.bereiche[${i}]`;
        if (b.art !== "text") {
          this.melde(pfad, "typ");
          return;
        }
        this.wohlgeformt(b.wert, pfad);
        if (!istSchluessel(b.wert)) this.melde(pfad, "muster");
        else if (gesehen.has(b.wert)) this.melde(pfad, "doppelt");
        gesehen.add(b.wert);
        k.bereiche.push(b.wert);
      });
    }
    const modus = this.text(o, "modus", "wissen.modus");
    if (modus !== undefined) {
      const treffer = MODI.find((m) => m === modus);
      if (treffer) k.modus = treffer;
      else this.melde("wissen.modus", "wert");
    }
    const kontexte = this.liste(o, "kontexte", "wissen.kontexte");
    if (kontexte) {
      if (kontexte.length > 50) this.melde("wissen.kontexte", "zu_viele");
      kontexte.forEach((e, i) => {
        const pfad = `wissen.kontexte[${i}]`;
        if (e.art !== "objekt") {
          this.melde(pfad, "typ");
          return;
        }
        const c = e.wert;
        this.pruefeSchluessel(c, pfad);
        const artText = this.text(c, "art", `${pfad}.art`, true);
        let art: Kontext["art"] = "buch";
        if (artText !== undefined) {
          const treffer = KONTEXT_ARTEN.find((a) => a === artText);
          if (treffer) art = treffer;
          else this.melde(`${pfad}.art`, "wert");
        }
        const id =
          this.begrenzt(
            this.text(c, "id", `${pfad}.id`, true),
            `${pfad}.id`,
            1,
            200,
          ) ?? "";
        const titel =
          this.begrenzt(
            this.text(c, "titel", `${pfad}.titel`),
            `${pfad}.titel`,
            0,
            200,
          ) ?? "";
        k.kontexte.push({
          art,
          id,
          titel,
          extra: this.unbekannte(c, ["art", "id", "titel"], pfad, 4),
        });
      });
    }
    k.extra = this.unbekannte(
      o,
      ["alle_bereiche", "bereiche", "modus", "kontexte"],
      "wissen",
      2,
    );
    return k;
  }

  schnittstellen(eintraege: JsonWert[]): string[] {
    if (eintraege.length > 32) this.melde("schnittstellen", "zu_viele");
    const gesehen = new Set<string>();
    const aus: string[] = [];
    eintraege.forEach((e, i) => {
      const pfad = `schnittstellen[${i}]`;
      if (e.art !== "text") {
        this.melde(pfad, "typ");
        return;
      }
      this.wohlgeformt(e.wert, pfad);
      if (!istSchnittstelle(e.wert)) this.melde(pfad, "muster");
      else if (gesehen.has(e.wert)) this.melde(pfad, "doppelt");
      gesehen.add(e.wert);
      aus.push(e.wert);
    });
    return aus;
  }

  auswertung(o: JsonMitglied[]): Auswertung {
    const a = neueAuswertung();
    this.pruefeSchluessel(o, "auswertung");
    const zusammenfassung = this.flagge(
      o,
      "zusammenfassung",
      "auswertung.zusammenfassung",
    );
    if (zusammenfassung !== undefined) a.zusammenfassung = zusammenfassung;
    const aufgaben = this.flagge(o, "aufgaben", "auswertung.aufgaben");
    if (aufgaben !== undefined) a.aufgaben = aufgaben;
    const felder = this.liste(o, "felder", "auswertung.felder");
    if (felder) {
      if (felder.length > 30) this.melde("auswertung.felder", "zu_viele");
      const namen = new Set<string>();
      felder.forEach((e, i) => {
        const pfad = `auswertung.felder[${i}]`;
        if (e.art !== "objekt") {
          this.melde(pfad, "typ");
          return;
        }
        const f = e.wert;
        this.pruefeSchluessel(f, pfad);
        const feld: AuswertungFeld = {
          name: "",
          beschreibung: "",
          typ: "text",
          optionen: [],
          extra: [],
        };
        const name = this.text(f, "name", `${pfad}.name`, true);
        if (name !== undefined) {
          if (!istSchluessel(name)) this.melde(`${pfad}.name`, "muster");
          else if (namen.has(name)) this.melde(`${pfad}.name`, "doppelt");
          namen.add(name);
          feld.name = name;
        }
        feld.beschreibung =
          this.begrenzt(
            this.text(f, "beschreibung", `${pfad}.beschreibung`),
            `${pfad}.beschreibung`,
            0,
            200,
          ) ?? "";
        let auswahl = false;
        const typText = this.text(f, "typ", `${pfad}.typ`, true);
        if (typText !== undefined) {
          const treffer = FELD_TYPEN.find((t) => t === typText);
          if (treffer) {
            feld.typ = treffer;
            auswahl = treffer === "auswahl";
          } else {
            this.melde(`${pfad}.typ`, "wert");
          }
        }
        const optionenPfad = `${pfad}.optionen`;
        const optionen = this.liste(f, "optionen", optionenPfad, auswahl);
        if (optionen) {
          if (auswahl) {
            if (optionen.length < 2) this.melde(optionenPfad, "laenge");
            if (optionen.length > 20) this.melde(optionenPfad, "zu_viele");
          }
          feld.optionen = this.texte(optionen, optionenPfad, 1, 60);
        }
        feld.extra = this.unbekannte(
          f,
          ["name", "beschreibung", "typ", "optionen"],
          pfad,
          4,
        );
        a.felder.push(feld);
      });
    }
    a.extra = this.unbekannte(
      o,
      ["zusammenfassung", "aufgaben", "felder"],
      "auswertung",
      2,
    );
    return a;
  }
}

/** Prüft einen JSON-Baum und liefert das Profil oder alle Fehler (sortiert, ohne Wiederholung). */
export function profilAusBaum(baum: JsonWert): ProfilErgebnis {
  if (baum.art !== "objekt")
    return { ok: false, fehler: [{ pfad: "", code: "json" }] };
  const pruefer = new Pruefer();
  const profil = pruefer.profil(baum.wert);
  if (pruefer.fehler.length === 0 && profil) return { ok: true, profil };
  return { ok: false, fehler: ordneFehler(pruefer.fehler) };
}

/** Liest und prüft einen Profiltext. Kein gültiges JSON oder kein Objekt: ein Fehler mit Pfad `""` und Code `json`. */
export function parseProfil(text: string): ProfilErgebnis {
  const baum = parseJson(text);
  if (baum === null) return { ok: false, fehler: [{ pfad: "", code: "json" }] };
  return profilAusBaum(baum);
}

/** Der JSON-Baum des Profils: unbekannte Felder, darüber die typisierten. */
export function profilBaum(p: Profil): JsonWert {
  const wurzel = kopiereMitglieder(p.extra);
  setzeMitglied(wurzel, "schema", jsonText(PROFIL_SCHEMA));
  setzeMitglied(wurzel, "id", jsonText(p.id));
  setzeMitglied(wurzel, "name", jsonText(p.name));
  setzeMitglied(wurzel, "beschreibung", jsonText(p.beschreibung));
  setzeMitglied(wurzel, "anweisungen", jsonText(p.anweisungen));
  setzeMitglied(wurzel, "begruessung", jsonText(p.begruessung));
  setzeMitglied(wurzel, "sprache", jsonText(p.sprache));
  setzeMitglied(
    wurzel,
    "stimme",
    p.stimme === null ? { art: "null" } : jsonText(p.stimme),
  );

  const v = kopiereMitglieder(p.verhalten.extra);
  setzeMitglied(v, "stil", jsonText(p.verhalten.stil));
  setzeMitglied(v, "sprechpause", jsonText(p.verhalten.sprechpause));
  setzeMitglied(v, "unterbrechen", jsonBool(p.verhalten.unterbrechen));
  setzeMitglied(
    v,
    "nachfrage_stille_s",
    jsonGanzzahl(p.verhalten.nachfrageStilleS),
  );
  setzeMitglied(wurzel, "verhalten", jsonObjekt(v));

  const k = kopiereMitglieder(p.wissen.extra);
  setzeMitglied(k, "alle_bereiche", jsonBool(p.wissen.alleBereiche));
  setzeMitglied(k, "bereiche", jsonTexte(p.wissen.bereiche));
  setzeMitglied(k, "modus", jsonText(p.wissen.modus));
  setzeMitglied(
    k,
    "kontexte",
    jsonListe(
      p.wissen.kontexte.map((c) => {
        const o = kopiereMitglieder(c.extra);
        setzeMitglied(o, "art", jsonText(c.art));
        setzeMitglied(o, "id", jsonText(c.id));
        setzeMitglied(o, "titel", jsonText(c.titel));
        return jsonObjekt(o);
      }),
    ),
  );
  setzeMitglied(wurzel, "wissen", jsonObjekt(k));

  setzeMitglied(wurzel, "schnittstellen", jsonTexte(p.schnittstellen));

  const e = kopiereMitglieder(p.auswertung.extra);
  setzeMitglied(e, "zusammenfassung", jsonBool(p.auswertung.zusammenfassung));
  setzeMitglied(e, "aufgaben", jsonBool(p.auswertung.aufgaben));
  setzeMitglied(
    e,
    "felder",
    jsonListe(
      p.auswertung.felder.map((f) => {
        const o = kopiereMitglieder(f.extra);
        setzeMitglied(o, "name", jsonText(f.name));
        setzeMitglied(o, "beschreibung", jsonText(f.beschreibung));
        setzeMitglied(o, "typ", jsonText(f.typ));
        setzeMitglied(o, "optionen", jsonTexte(f.optionen));
        return jsonObjekt(o);
      }),
    ),
  );
  setzeMitglied(wurzel, "auswertung", jsonObjekt(e));
  return jsonObjekt(wurzel);
}

/** Der kanonische Text (sortierte Schlüssel, 2 Leerzeichen, `\n` am Ende), bytegleich zur Swift-Seite. */
export function kodiereProfil(p: Profil): string {
  return kodiereJson(profilBaum(p));
}

/** Prüft das Profil in seinem heutigen Stand (nach Änderungen an den typisierten Feldern). */
export function pruefeProfil(p: Profil): ProfilFehler[] {
  const ergebnis = profilAusBaum(profilBaum(p));
  return ergebnis.ok ? [] : ergebnis.fehler;
}

// ---------------------------------------------------------------- Bereiche und Kontextmodus

export type PcBereich = (typeof ALL_AREAS)[number];

/** Abbildung der Profil-Schlüssel auf die Bereiche des PC; `ablage` hat dort noch keine Wirkung. */
const PC_ABBILDUNG: ReadonlyArray<readonly [string, PcBereich]> = [
  ["buecher", "books"],
  ["buchnotizen", "book_notes"],
  ["ki_dokumente", "ki_docs"],
  ["besprechungen", "meetings"],
  ["sprachnotizen", "dictations"],
  ["gespraeche", "conversations"],
  ["kalender", "calendar"],
  ["vorlesen", "tts"],
  ["notizbuch", "notebook"],
];

/**
 * Wirksame Bereiche = (alle Bereiche des PC ODER die abgebildeten Schlüssel des Profils) ∩ freigegebene. Das Profil öffnet nie
 * mehr, als freigegeben ist. Reihenfolge wie `ALL_AREAS`.
 */
export function wirksameBereiche(
  profil: Profil,
  freigegeben: readonly string[],
): PcBereich[] {
  const gewollt = new Set<PcBereich>(
    profil.wissen.alleBereiche
      ? ALL_AREAS
      : PC_ABBILDUNG.filter(([schluessel]) =>
          profil.wissen.bereiche.includes(schluessel),
        ).map(([, bereich]) => bereich),
  );
  return ALL_AREAS.filter((b) => gewollt.has(b) && freigegeben.includes(b));
}

/** `zuerst`/`nur` ohne Kontexte wirkt wie `alles`. */
export function kontextModusWirksam(profil: Profil): Wissen["modus"] {
  return profil.wissen.modus !== "alles" && profil.wissen.kontexte.length === 0
    ? "alles"
    : profil.wissen.modus;
}
