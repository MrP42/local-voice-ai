/**
 * Auswertung am Gesprächsende (lva#559, Paket G5-Kern): Prompt für EIN Modell, tolerantes Lesen der Rohantwort mit Erdung im Code
 * und das Ergebnisformat `lva.auswertung/1`. Reine Funktionen ohne Oberfläche, ohne Modellaufruf und ohne Speicher; die Auswertung
 * schreibt nie in Logs, Hinweise tragen nur Pfad und Code. Gegenstück: `AssistantOutcome*.swift`. Regeln: README „Auswertung“,
 * gemeinsame Prüfstände: `apps/shared/assistenten/auswertung-pruefstaende.json` (beide Kerne bestehen alle Fälle bytegleich).
 */
import {
  FELD_TYPEN,
  codepunkte,
  istProfilId,
  istSchluessel,
  jsonBool,
  jsonGanzzahl,
  jsonText,
  kodiereJson,
  mitglied,
  ordneFehler,
  parseJson,
  setzeMitglied,
  vergleicheCodepunkte,
  zaehle,
  type AuswertungFeld,
  type FehlerCode,
  type JsonMitglied,
  type JsonWert,
  type Profil,
  type ProfilFehler,
} from "./profil";
import {
  type Datum,
  ausCodepunkten,
  einzeilig,
  fristenIn,
  frist,
  formatiereDatum,
  hatZahlwort,
  istGueltigesDatum,
  istLeerraum,
  kappeLeerraum,
  liesZahl,
  normalisiere,
  parseIsoDatum,
  wochentag,
  zahlenIn,
} from "./auswertungFrist";

// ---------------------------------------------------------------- Typen

export const AUSWERTUNG_SCHEMA = "lva.auswertung/1";
export const FELD_STATI = ["klar", "unklar", "nicht_genannt"] as const;
export type FeldStatus = (typeof FELD_STATI)[number];
export type FeldTyp = (typeof FELD_TYPEN)[number];

export type Rolle = "nutzer" | "assistent";
/** Ein Beitrag des Gesprächs; die Nummer ist die Position (ab 1) im übergebenen Verlauf. */
export type Beitrag = { rolle: Rolle; text: string };

export type FeldErgebnis = {
  name: string;
  typ: FeldTyp;
  status: FeldStatus;
  wert: string | null;
  beleg: string | null;
  beitrag: number | null;
};
export type AufgabeErgebnis = {
  text: string;
  /** `JJJJ-MM-TT`, nur wenn die Fristauflösung `fristText` auflöst. */
  frist: string | null;
  fristText: string;
  beleg: string;
  beitrag: number;
};
export type AuswertungErgebnis = {
  assistent: string;
  /** ISO-8601 UTC (`JJJJ-MM-TTThh:mm:ssZ`), vom Aufrufer; leer, solange er nichts gesetzt hat. */
  erstellt: string;
  gekuerzt: boolean;
  zusammenfassung: string;
  felder: FeldErgebnis[];
  aufgaben: AufgabeErgebnis[];
  offeneFragen: string[];
};

export const HINWEIS_CODES = [
  "beleg_fehlt",
  "wert_nicht_im_beleg",
  "typ",
  "wert",
  "unbekannt",
  "doppelt",
  "laenge",
  "fehlt",
  "zu_viele",
] as const;
export type HinweisCode = (typeof HINWEIS_CODES)[number];
/** Ein Hinweis der Auswertung: nur Pfad und Code, nie Inhalt. */
export type AuswertungHinweis = { pfad: string; code: HinweisCode };

export type AuswertenErgebnis =
  | { ok: true; ergebnis: AuswertungErgebnis; hinweise: AuswertungHinweis[] }
  | { ok: false; fehler: "json" };

export type AuswertenOptionen = { erstellt?: string; gekuerzt?: boolean };

export const MAX_ZUSAMMENFASSUNG = 1200;
export const MAX_AUFGABEN = 20;
export const MAX_AUFGABE_TEXT = 300;
export const MAX_FRIST_TEXT = 100;
export const MAX_FRAGEN = 10;
export const MAX_FRAGE = 300;
export const MAX_WERT = 500;
export const MAX_BELEG = 500;
/** Längere Rohantworten (Codepunkte) sind der Fehler `json`, ohne weitere Arbeit. */
export const MAX_ROHANTWORT = 262144;
/** So viele `{` ohne schließende Klammer werden höchstens ausprobiert; das nächste beendet die Suche mit `json`. */
export const MAX_OFFENE_STARTS = 16;

// ---------------------------------------------------------------- Text-Helfer

/** Erste `max` Codepunkte, Leerraum am Ende entfernt. */
function kappeHart(s: string, max: number): string {
  const c = codepunkte(s);
  if (c.length <= max) return s;
  let ende = max;
  while (ende > 0 && istLeerraum(c[ende - 1])) ende -= 1;
  return ausCodepunkten(c, 0, ende);
}

/** Kürzt am letzten Satzende (`.`, `!`, `?` vor Leerraum oder Textende) innerhalb von `max` Codepunkten, sonst hart. */
function kappeAmSatzende(s: string, max: number): string {
  const c = codepunkte(s);
  if (c.length <= max) return s;
  for (let k = max - 1; k >= 0; k -= 1) {
    const satzende = c[k] === 0x2e || c[k] === 0x21 || c[k] === 0x3f;
    if (satzende && (k + 1 === c.length || istLeerraum(c[k + 1]))) {
      return ausCodepunkten(c, 0, k + 1);
    }
  }
  return kappeHart(s, max);
}

// ---------------------------------------------------------------- Prompt

export type AuswertungPrompt = { text: string; gekuerzt: boolean };

const WOCHENTAGE_DE = [
  "Montag",
  "Dienstag",
  "Mittwoch",
  "Donnerstag",
  "Freitag",
  "Samstag",
  "Sonntag",
];

function typBeschreibung(f: AuswertungFeld): string {
  switch (f.typ) {
    case "text":
      return "Text";
    case "zahl":
      return "Zahl, mit Punkt als Dezimaltrenner, ohne Tausenderzeichen";
    case "datum":
      return "Datum als JJJJ-MM-TT, relative Angaben anhand des heutigen Datums umgerechnet";
    case "auswahl":
      return `Auswahl, genau eine von: ${f.optionen
        .map((o) => `"${einzeilig(o)}"`)
        .join(", ")}`;
  }
}

/** Die Anweisung an das Modell (alles bis einschließlich `Gespräch:`), nur für das, was das Profil einschaltet. */
function anweisung(profil: Profil, heute: Datum): string[] {
  const a = profil.auswertung;
  const felder = a.felder.length > 0;
  const zeilen: string[] = [
    "Werte das folgende beendete Gespräch aus. Antworte mit genau einem JSON-Objekt und sonst nichts: kein Text davor oder danach, keine Code-Zäune, keine Erklärung.",
    "",
  ];
  if (istGueltigesDatum(heute)) {
    zeilen.push(
      `Heutiges Datum: ${formatiereDatum(heute)} (${WOCHENTAGE_DE[wochentag(heute) - 1]}).`,
      "",
    );
  }
  zeilen.push(
    "Regeln:",
    "- Erfinde nichts. Nimm nur auf, was im Gespräch gesagt wurde.",
  );
  if (felder || a.aufgaben) {
    zeilen.push(
      '- Jeder Beleg ist ein wörtliches Zitat aus dem Gespräch, "beitrag" ist die Nummer des Beitrags, aus dem es stammt.',
    );
  }
  if (felder) {
    zeilen.push(
      '- Je Feld gilt ein Status: "klar" (eindeutig genannt), "unklar" (genannt, aber unsicher, widersprüchlich oder nur zum Teil) oder "nicht_genannt" (kommt im Gespräch nicht vor). Bei "nicht_genannt" sind "wert", "beleg" und "beitrag" null.',
      '- Widersprechen sich Angaben oder wurde etwas später korrigiert, gilt die letzte eindeutige Angabe, sonst ist der Status "unklar".',
    );
  }
  if (a.aufgaben) {
    zeilen.push(
      '- Fristen stehen als wörtlicher Ausdruck aus dem Gespräch in "frist_text" (zum Beispiel "nächsten Dienstag"), nicht als errechnetes Datum.',
    );
  }
  zeilen.push(
    "- Verwende nur Namen, die im Gespräch vorkommen; erfinde keinen Ersatznamen. Keine Floskeln.",
    "- Schreibe in der Sprache des Gesprächs.",
    "",
    "Das JSON-Objekt hat genau diese Schlüssel:",
  );
  if (felder) {
    zeilen.push(
      '- "felder": Liste mit genau einem Eintrag je Feld, in dieser Reihenfolge. Jeder Eintrag hat die Schlüssel "name", "status", "wert" (Text oder null), "beleg" (Zitat oder null) und "beitrag" (Zahl oder null). Die Felder:',
    );
    for (const f of a.felder) {
      const beschreibung = einzeilig(f.beschreibung);
      zeilen.push(
        `  - ${f.name} (${typBeschreibung(f)})${beschreibung === "" ? "" : `: ${beschreibung}`}`,
      );
    }
  }
  if (a.zusammenfassung) {
    zeilen.push(
      '- "zusammenfassung": Zusammenfassung des Gesprächs in zwei bis fünf Sätzen (Text).',
    );
  }
  if (a.aufgaben) {
    zeilen.push(
      '- "aufgaben": Liste der Aufgaben und Zusagen aus dem Gespräch, leer, wenn es keine gibt. Jeder Eintrag hat die Schlüssel "text" (kurz), "frist_text" (wörtlicher Ausdruck oder leerer Text), "beleg" (Zitat) und "beitrag" (Zahl).',
    );
  }
  zeilen.push(
    '- "offene_fragen": Liste von Fragen oder Punkten, die im Gespräch offen blieben (Texte), leer, wenn es keine gibt.',
    "",
    "Gespräch:",
  );
  return zeilen;
}

/** Die Gesprächsfassung: ein Beitrag je Zeile, `[n] Nutzer: …` oder `[n] Assistent: …`, Leerraum zu einem Leerzeichen zusammengefasst. */
export function gespraechZeilen(verlauf: readonly Beitrag[]): string[] {
  return verlauf.map((b, i) => {
    const text = einzeilig(b.text);
    return `[${i + 1}] ${b.rolle === "nutzer" ? "Nutzer" : "Assistent"}:${text === "" ? "" : ` ${text}`}`;
  });
}

const auslassung = (n: number) =>
  `[… ${n} ${n === 1 ? "Beitrag" : "Beiträge"} ausgelassen …]`;

/**
 * Der Prompt für EIN Modell: Anweisung und Gespräch. `budgetZeichen` zählt Codepunkte des ganzen Textes. Passt er nicht, bleiben die
 * ersten zwei Beiträge und so viele der neuesten wie möglich; dazwischen steht `[… n Beiträge ausgelassen …]` (die Nummern
 * bleiben die des ungekürzten Gesprächs), `gekuerzt` ist dann wahr. Ein einzelner Beitrag wird nie geschnitten; das Mindeste
 * (zwei erste Beiträge und die Auslassung) bleibt auch über dem Budget stehen. Mit höchstens zwei Beiträgen ist nichts auszulassen.
 */
export function auswertungPrompt(
  profil: Profil,
  verlauf: readonly Beitrag[],
  heute: Datum,
  budgetZeichen: number,
): AuswertungPrompt {
  const kopf = anweisung(profil, heute).join("\n");
  const zeilen = gespraechZeilen(verlauf);
  const voll = [kopf].concat(zeilen).join("\n");
  const budget = Math.max(0, budgetZeichen);
  if (zaehle(voll) <= budget || zeilen.length <= 2) {
    return { text: voll, gekuerzt: false };
  }
  const laengen = zeilen.map((z) => zaehle(z));
  const kopfLaenge = zaehle(kopf);
  const n = zeilen.length;
  // mit k neuesten Beiträgen: Kopf + 2 erste + Auslassung + k neueste, je durch einen Zeilenumbruch getrennt
  const laenge = (k: number, summeNeueste: number) =>
    kopfLaenge +
    1 +
    laengen[0] +
    1 +
    laengen[1] +
    1 +
    zaehle(auslassung(n - 2 - k)) +
    (k > 0 ? summeNeueste + k : 0);
  let k = 0;
  let summe = 0;
  while (k < n - 3) {
    const naechste = summe + laengen[n - 1 - k];
    if (laenge(k + 1, naechste) > budget) break;
    summe = naechste;
    k += 1;
  }
  const behalten = [kopf, zeilen[0], zeilen[1], auslassung(n - 2 - k)].concat(
    zeilen.slice(n - k),
  );
  return { text: behalten.join("\n"), gekuerzt: true };
}

// ---------------------------------------------------------------- Rohantwort lesen

/** Index der schließenden Klammer des Objekts ab `von` (Zeichenketten und Escapes beachtet), sonst -1. */
function ausgeglichenBis(text: string, von: number): number {
  let tiefe = 0;
  let imText = false;
  let escape = false;
  for (let i = von; i < text.length; i += 1) {
    const c = text[i];
    if (imText) {
      if (escape) escape = false;
      else if (c === "\\") escape = true;
      else if (c === '"') imText = false;
      continue;
    }
    if (c === '"') imText = true;
    else if (c === "{") tiefe += 1;
    else if (c === "}") {
      tiefe -= 1;
      if (tiefe === 0) return i;
    }
  }
  return -1;
}

/** Mehr als `max` Codepunkte? Zählt ohne Kopie und hört beim Überschreiten auf. */
function laengerAls(s: string, max: number): boolean {
  let n = 0;
  for (let i = 0; i < s.length; i += 1) {
    const c = s.charCodeAt(i);
    const hoch = c >= 0xd800 && c <= 0xdbff;
    const tief =
      i + 1 < s.length &&
      s.charCodeAt(i + 1) >= 0xdc00 &&
      s.charCodeAt(i + 1) <= 0xdfff;
    if (hoch && tief) i += 1;
    n += 1;
    if (n > max) return true;
  }
  return false;
}

/**
 * Das erste ausbalancierte, gültige JSON-Objekt im Text (auch in Code-Zäunen oder mit Text davor und danach). Grenzen: mehr als
 * `MAX_ROHANTWORT` Codepunkte sind `null`; ein `{` ohne schließende Klammer zählt als offener Start, der 17. beendet die Suche
 * (sonst wäre sie quadratisch); die Tiefe begrenzt der strenge JSON-Leser wie beim Profil (64 Ebenen).
 */
export function ersterObjektBaum(text: string): JsonMitglied[] | null {
  if (laengerAls(text, MAX_ROHANTWORT)) return null;
  let offeneStarts = 0;
  let von = text.indexOf("{");
  while (von >= 0) {
    const ende = ausgeglichenBis(text, von);
    if (ende < 0) {
      offeneStarts += 1;
      if (offeneStarts > MAX_OFFENE_STARTS) return null;
      von = text.indexOf("{", von + 1);
      continue;
    }
    const baum = parseJson(text.slice(von, ende + 1));
    if (baum !== null && baum.art === "objekt") return baum.wert;
    von = text.indexOf("{", ende + 1);
  }
  return null;
}

// ---------------------------------------------------------------- Hinweise

class Hinweise {
  private readonly liste: AuswertungHinweis[] = [];
  melde(pfad: string, code: HinweisCode): void {
    this.liste.push({ pfad, code });
  }
  /** Unter-Pfad; ein Schlüssel, der nicht dem Schlüsselmuster entspricht (Inhalt des Modells), erscheint nie. */
  static kind(eltern: string, schluessel: string): string {
    if (!istSchluessel(schluessel)) return eltern;
    return eltern === "" ? schluessel : `${eltern}.${schluessel}`;
  }
  geordnet(): AuswertungHinweis[] {
    const gesehen = new Set<string>();
    return this.liste
      .filter((h) => {
        const k = `${h.pfad}\u0000${h.code}`;
        if (gesehen.has(k)) return false;
        gesehen.add(k);
        return true;
      })
      .sort(
        (a, b) =>
          vergleicheCodepunkte(a.pfad, b.pfad) ||
          vergleicheCodepunkte(a.code, b.code),
      );
  }
}

type Kontext = {
  /** Normalisierte Beiträge des ungekürzten Gesprächs. */
  normiert: string[];
  heute: Datum;
  h: Hinweise;
};

function meldeUnbekannte(
  o: JsonMitglied[],
  bekannt: readonly string[],
  pfad: string,
  h: Hinweise,
): void {
  for (const m of o) {
    if (!bekannt.includes(m.schluessel)) {
      h.melde(Hinweise.kind(pfad, m.schluessel), "unbekannt");
    }
  }
}

/** Text (oder Zahl als Originaltext) eines Rohwerts, an den Enden von Leerraum befreit; leer und `null` ergeben `null`. */
function liesText(
  w: JsonWert | undefined,
  h: Hinweise,
  pfad: string,
  auchZahl: boolean,
): string | null {
  if (w === undefined || w.art === "null") return null;
  if (w.art === "text") {
    const t = kappeLeerraum(w.wert);
    return t === "" ? null : t;
  }
  if (w.art === "zahl" && auchZahl) return w.roh;
  h.melde(pfad, "typ");
  return null;
}

function liesBeitrag(
  w: JsonWert | undefined,
  h: Hinweise,
  pfad: string,
): number | null {
  if (w === undefined || w.art === "null") return null;
  if (w.art === "zahl" && /^(0|[1-9][0-9]{0,8})$/.test(w.roh))
    return Number(w.roh);
  if (w.art === "text" && /^[0-9]{1,6}$/.test(kappeLeerraum(w.wert))) {
    return Number(kappeLeerraum(w.wert));
  }
  h.melde(pfad, "typ");
  return null;
}

const ROLLEN_PRAEFIX = ["nutzer", "assistent", "user", "assistant"];

/**
 * Streift ein Beitragspräfix vom Beleg: `[3] Nutzer: …`, `[3] …`, `Nutzer: …`, `Assistent: …`, `User: …`, `Assistant: …` (Rolle ohne
 * Beachtung der Groß-/Kleinschreibung, Leerraum um Nummer und Doppelpunkt frei). Die Nummer (1 bis 6 Ziffern in eckigen Klammern)
 * wird mitgeliefert. Ohne Präfix bleibt der Text unverändert.
 */
export function entfernePraefix(beleg: string): {
  text: string;
  beitrag: number | null;
} {
  const c = codepunkte(beleg);
  let i = 0;
  let nummer: number | null = null;
  const leerraum = () => {
    while (i < c.length && istLeerraum(c[i])) i += 1;
  };
  leerraum();
  if (i < c.length && c[i] === 0x5b) {
    let j = i + 1;
    while (j < c.length && c[j] >= 0x30 && c[j] <= 0x39) j += 1;
    if (j > i + 1 && j - (i + 1) <= 6 && j < c.length && c[j] === 0x5d) {
      nummer = Number(ausCodepunkten(c, i + 1, j));
      i = j + 1;
      leerraum();
    }
  }
  let j = i;
  while (
    j < c.length &&
    j - i <= 9 &&
    ((c[j] >= 0x41 && c[j] <= 0x5a) || (c[j] >= 0x61 && c[j] <= 0x7a))
  ) {
    j += 1;
  }
  if (j > i && j - i <= 9) {
    const wort = ausCodepunkten(c, i, j).toLowerCase();
    let k = j;
    while (k < c.length && istLeerraum(c[k])) k += 1;
    if (ROLLEN_PRAEFIX.includes(wort) && k < c.length && c[k] === 0x3a) {
      i = k + 1;
      leerraum();
    }
  }
  if (i === 0 && nummer === null) return { text: beleg, beitrag: null };
  return { text: ausCodepunkten(c, i), beitrag: nummer };
}

/** Beleg der Rohantwort: Präfix ab, auf `MAX_BELEG` gekürzt; die Nummer des Präfixes gilt, wenn die Antwort keinen Beitrag nennt. */
function liesBeleg(
  roh: string | null,
  rohBeitrag: number | null,
): { text: string | null; gekappt: boolean; beitrag: number | null } {
  if (roh === null) return { text: null, gekappt: false, beitrag: rohBeitrag };
  const ohne = entfernePraefix(roh);
  const beitrag = rohBeitrag ?? ohne.beitrag;
  if (ohne.text === "") return { text: null, gekappt: false, beitrag };
  const kurz = kappeHart(ohne.text, MAX_BELEG);
  return { text: kurz, gekappt: kurz !== ohne.text, beitrag };
}

/** Nummer des Beitrags, in dem der (normalisierte) Beleg steht: der angegebene, sonst der erste, der ihn enthält. */
function findeBeleg(
  beleg: string,
  beitrag: number | null,
  k: Kontext,
): number | null {
  const n = normalisiere(beleg);
  if (n === "") return null;
  if (beitrag !== null && beitrag >= 1 && beitrag <= k.normiert.length) {
    if (k.normiert[beitrag - 1].includes(n)) return beitrag;
  }
  const i = k.normiert.findIndex((t) => t.includes(n));
  return i < 0 ? null : i + 1;
}

// ---------------------------------------------------------------- Felder

const leeresFeld = (f: AuswertungFeld): FeldErgebnis => ({
  name: f.name,
  typ: f.typ,
  status: "nicht_genannt",
  wert: null,
  beleg: null,
  beitrag: null,
});

type Kanonisch = {
  wert: string;
  typfehler: HinweisCode | null;
  gedeckt: boolean;
};

/** Kanonischer Wert nach Typ und ob der Beleg ihn deckt (siehe README). */
function kanonisch(
  f: AuswertungFeld,
  rohWert: string,
  beleg: string,
  k: Kontext,
  pfad: string,
): Kanonisch {
  const nBeleg = normalisiere(beleg);
  const fehler = (code: HinweisCode): Kanonisch => {
    k.h.melde(pfad, code);
    return {
      wert: kappeHart(rohWert, MAX_WERT),
      typfehler: code,
      gedeckt: false,
    };
  };
  switch (f.typ) {
    case "text": {
      const wert = kappeHart(rohWert, MAX_WERT);
      if (wert !== rohWert) k.h.melde(pfad, "laenge");
      const n = normalisiere(wert);
      return { wert, typfehler: null, gedeckt: n !== "" && nBeleg.includes(n) };
    }
    case "zahl": {
      const z = liesZahl(rohWert);
      if (z === null) return fehler("typ");
      const ganz = /^[0-9]+$/.test(z.vergleich) && Number(z.vergleich) <= 12;
      const gedeckt =
        zahlenIn(nBeleg).includes(z.vergleich) ||
        (ganz && hatZahlwort(nBeleg, Number(z.vergleich)));
      return { wert: z.kanonisch, typfehler: null, gedeckt };
    }
    case "datum": {
      const direkt = parseIsoDatum(rohWert);
      const text = direkt ? formatiereDatum(direkt) : frist(rohWert, k.heute);
      if (text === null) return fehler("typ");
      return {
        wert: text,
        typfehler: null,
        gedeckt: fristenIn(beleg, k.heute).includes(text),
      };
    }
    case "auswahl": {
      const n = normalisiere(rohWert);
      const option =
        f.optionen.find((o) => o === rohWert) ??
        f.optionen.find((o) => n !== "" && normalisiere(o) === n);
      if (option === undefined) return fehler("wert");
      const nOption = normalisiere(option);
      return {
        wert: option,
        typfehler: null,
        gedeckt: nOption !== "" && nBeleg.includes(nOption),
      };
    }
  }
}

function werteFeld(
  f: AuswertungFeld,
  p: number,
  roh: JsonMitglied[],
  k: Kontext,
): FeldErgebnis {
  const pfad = `felder[${p}]`;
  meldeUnbekannte(
    roh,
    ["name", "status", "wert", "beleg", "beitrag"],
    pfad,
    k.h,
  );
  const rohWert = liesText(mitglied(roh, "wert"), k.h, `${pfad}.wert`, true);
  const rohBeleg = liesText(
    mitglied(roh, "beleg"),
    k.h,
    `${pfad}.beleg`,
    false,
  );
  const rohBeitrag = liesBeitrag(
    mitglied(roh, "beitrag"),
    k.h,
    `${pfad}.beitrag`,
  );

  const s = mitglied(roh, "status");
  let status: FeldStatus = rohWert === null ? "nicht_genannt" : "klar";
  if (s === undefined) {
    k.h.melde(`${pfad}.status`, "fehlt");
  } else if (
    s.art === "text" &&
    (FELD_STATI as readonly string[]).includes(s.wert)
  ) {
    status = s.wert as FeldStatus;
  } else {
    k.h.melde(`${pfad}.status`, s.art === "text" ? "wert" : "typ");
  }
  if (status === "nicht_genannt") return leeresFeld(f);
  if (rohWert === null && status === "klar") return leeresFeld(f);

  const gelesen = liesBeleg(rohBeleg, rohBeitrag);
  const beleg = gelesen.text;
  if (gelesen.gekappt) k.h.melde(`${pfad}.beleg`, "laenge");
  if (rohWert === null && beleg === null) {
    return { ...leeresFeld(f), status: "unklar" };
  }
  const ort = beleg === null ? null : findeBeleg(beleg, gelesen.beitrag, k);
  if (beleg === null || ort === null) {
    k.h.melde(`${pfad}.beleg`, "beleg_fehlt");
    return leeresFeld(f);
  }
  if (rohWert === null) {
    return { ...leeresFeld(f), status: "unklar", beleg, beitrag: ort };
  }
  const c = kanonisch(f, rohWert, beleg, k, `${pfad}.wert`);
  if (c.typfehler === null && !c.gedeckt)
    k.h.melde(`${pfad}.wert`, "wert_nicht_im_beleg");
  const klar = status === "klar" && c.typfehler === null && c.gedeckt;
  return {
    name: f.name,
    typ: f.typ,
    status: klar ? "klar" : "unklar",
    wert: c.wert,
    beleg,
    beitrag: ort,
  };
}

function werteFelder(
  profil: Profil,
  wurzel: JsonMitglied[],
  k: Kontext,
): FeldErgebnis[] {
  const profilFelder = profil.auswertung.felder;
  const w = mitglied(wurzel, "felder");
  let eintraege: JsonWert[] = [];
  if (w !== undefined && w.art !== "null") {
    if (w.art !== "liste") k.h.melde("felder", "typ");
    else eintraege = w.wert;
  }
  if (profilFelder.length === 0) {
    if (eintraege.length > 0) k.h.melde("felder", "unbekannt");
    return [];
  }
  const gefunden = new Map<string, JsonMitglied[]>();
  eintraege.forEach((e, i) => {
    if (e.art !== "objekt") {
      k.h.melde(`felder[${i}]`, "typ");
      return;
    }
    const name = mitglied(e.wert, "name");
    if (name === undefined || name.art !== "text") {
      k.h.melde(`felder[${i}]`, name === undefined ? "fehlt" : "typ");
      return;
    }
    if (!profilFelder.some((f) => f.name === name.wert)) {
      k.h.melde(`felder[${i}]`, "unbekannt");
      return;
    }
    if (gefunden.has(name.wert)) {
      k.h.melde(`felder[${i}]`, "doppelt");
      return;
    }
    gefunden.set(name.wert, e.wert);
  });
  return profilFelder.map((f, p) => {
    const roh = gefunden.get(f.name);
    return roh === undefined ? leeresFeld(f) : werteFeld(f, p, roh, k);
  });
}

// ---------------------------------------------------------------- Aufgaben, Zusammenfassung, Fragen

function werteAufgaben(
  profil: Profil,
  wurzel: JsonMitglied[],
  k: Kontext,
): AufgabeErgebnis[] {
  const w = mitglied(wurzel, "aufgaben");
  if (w === undefined || w.art === "null") return [];
  if (w.art !== "liste") {
    k.h.melde("aufgaben", "typ");
    return [];
  }
  if (!profil.auswertung.aufgaben) {
    if (w.wert.length > 0) k.h.melde("aufgaben", "unbekannt");
    return [];
  }
  const aus: AufgabeErgebnis[] = [];
  let zuViele = false;
  w.wert.forEach((e, i) => {
    const pfad = `aufgaben[${i}]`;
    if (e.art !== "objekt") {
      k.h.melde(pfad, "typ");
      return;
    }
    meldeUnbekannte(
      e.wert,
      ["text", "frist_text", "beleg", "beitrag"],
      pfad,
      k.h,
    );
    const text = liesText(mitglied(e.wert, "text"), k.h, `${pfad}.text`, false);
    const rohFrist = liesText(
      mitglied(e.wert, "frist_text"),
      k.h,
      `${pfad}.frist_text`,
      false,
    );
    const rohBeleg = liesText(
      mitglied(e.wert, "beleg"),
      k.h,
      `${pfad}.beleg`,
      false,
    );
    const rohBeitrag = liesBeitrag(
      mitglied(e.wert, "beitrag"),
      k.h,
      `${pfad}.beitrag`,
    );
    if (text === null) {
      k.h.melde(`${pfad}.text`, "fehlt");
      return;
    }
    const gelesen = liesBeleg(rohBeleg, rohBeitrag);
    const beleg = gelesen.text;
    const ort = beleg === null ? null : findeBeleg(beleg, gelesen.beitrag, k);
    if (beleg === null || ort === null) {
      k.h.melde(`${pfad}.beleg`, "beleg_fehlt");
      return;
    }
    if (aus.length >= MAX_AUFGABEN) {
      zuViele = true;
      return;
    }
    const kurz = kappeAmSatzende(text, MAX_AUFGABE_TEXT);
    if (kurz !== text) k.h.melde(`${pfad}.text`, "laenge");
    let fristText =
      rohFrist === null ? "" : kappeHart(rohFrist, MAX_FRIST_TEXT);
    if (fristText !== "") {
      const n = normalisiere(fristText);
      if (n === "" || !k.normiert.some((t) => t.includes(n))) {
        k.h.melde(`${pfad}.frist_text`, "beleg_fehlt");
        fristText = "";
      }
    }
    aus.push({
      text: kurz,
      frist: fristText === "" ? null : frist(fristText, k.heute),
      fristText,
      beleg,
      beitrag: ort,
    });
  });
  if (zuViele) k.h.melde("aufgaben", "zu_viele");
  return aus;
}

function werteZusammenfassung(
  profil: Profil,
  wurzel: JsonMitglied[],
  k: Kontext,
): string {
  const w = mitglied(wurzel, "zusammenfassung");
  if (w === undefined || w.art === "null") return "";
  if (w.art !== "text") {
    k.h.melde("zusammenfassung", "typ");
    return "";
  }
  const t = kappeLeerraum(w.wert);
  if (!profil.auswertung.zusammenfassung) {
    if (t !== "") k.h.melde("zusammenfassung", "unbekannt");
    return "";
  }
  const kurz = kappeAmSatzende(t, MAX_ZUSAMMENFASSUNG);
  if (kurz !== t) k.h.melde("zusammenfassung", "laenge");
  return kurz;
}

function werteFragen(wurzel: JsonMitglied[], k: Kontext): string[] {
  const w = mitglied(wurzel, "offene_fragen");
  if (w === undefined || w.art === "null") return [];
  if (w.art !== "liste") {
    k.h.melde("offene_fragen", "typ");
    return [];
  }
  const aus: string[] = [];
  let zuViele = false;
  w.wert.forEach((e, i) => {
    const pfad = `offene_fragen[${i}]`;
    if (e.art !== "text") {
      k.h.melde(pfad, "typ");
      return;
    }
    const t = kappeLeerraum(e.wert);
    if (t === "") return;
    if (aus.length >= MAX_FRAGEN) {
      zuViele = true;
      return;
    }
    const kurz = kappeAmSatzende(t, MAX_FRAGE);
    if (kurz !== t) k.h.melde(pfad, "laenge");
    aus.push(kurz);
  });
  if (zuViele) k.h.melde("offene_fragen", "zu_viele");
  return aus;
}

// ---------------------------------------------------------------- Auswerten

/**
 * Liest die Rohantwort des Modells und erdet sie im Gespräch. Kein auffindbares JSON-Objekt: `{ok: false, fehler: "json"}` (der
 * Aufrufer kann einmal neu fragen). Sonst das Ergebnis `lva.auswertung/1` mit allen Feldern des Profils in dessen Reihenfolge
 * und die nach Pfad und Code sortierten Hinweise. `heute` ist der Tag des Gesprächs; `optionen` setzt `erstellt` und `gekuerzt`
 * (aus `auswertungPrompt`), die der Kern nicht kennt.
 */
export function auswerten(
  profil: Profil,
  verlauf: readonly Beitrag[],
  rohantwort: string,
  heute: Datum,
  optionen: AuswertenOptionen = {},
): AuswertenErgebnis {
  const wurzel = ersterObjektBaum(rohantwort);
  if (wurzel === null) return { ok: false, fehler: "json" };
  const h = new Hinweise();
  const k: Kontext = {
    normiert: verlauf.map((b) => normalisiere(b.text)),
    heute,
    h,
  };
  meldeUnbekannte(
    wurzel,
    ["felder", "zusammenfassung", "aufgaben", "offene_fragen"],
    "",
    h,
  );
  const ergebnis: AuswertungErgebnis = {
    assistent: profil.id,
    erstellt: optionen.erstellt ?? "",
    gekuerzt: optionen.gekuerzt ?? false,
    zusammenfassung: werteZusammenfassung(profil, wurzel, k),
    felder: werteFelder(profil, wurzel, k),
    aufgaben: werteAufgaben(profil, wurzel, k),
    offeneFragen: werteFragen(wurzel, k),
  };
  return { ok: true, ergebnis, hinweise: h.geordnet() };
}

// ---------------------------------------------------------------- Kodieren und Lesen

const jsonNull = (): JsonWert => ({ art: "null" });
const jsonTextOderNull = (s: string | null): JsonWert =>
  s === null ? jsonNull() : jsonText(s);
const jsonZahlOderNull = (n: number | null): JsonWert =>
  n === null ? jsonNull() : jsonGanzzahl(n);
const objekt = (wert: JsonMitglied[]): JsonWert => ({ art: "objekt", wert });
const liste = (wert: JsonWert[]): JsonWert => ({ art: "liste", wert });

/** Der JSON-Baum des Ergebnisses. */
export function auswertungBaum(e: AuswertungErgebnis): JsonWert {
  const wurzel: JsonMitglied[] = [];
  setzeMitglied(wurzel, "schema", jsonText(AUSWERTUNG_SCHEMA));
  setzeMitglied(wurzel, "assistent", jsonText(e.assistent));
  setzeMitglied(wurzel, "erstellt", jsonText(e.erstellt));
  setzeMitglied(wurzel, "gekuerzt", jsonBool(e.gekuerzt));
  setzeMitglied(wurzel, "zusammenfassung", jsonText(e.zusammenfassung));
  setzeMitglied(
    wurzel,
    "felder",
    liste(
      e.felder.map((f) => {
        const o: JsonMitglied[] = [];
        setzeMitglied(o, "name", jsonText(f.name));
        setzeMitglied(o, "typ", jsonText(f.typ));
        setzeMitglied(o, "status", jsonText(f.status));
        setzeMitglied(o, "wert", jsonTextOderNull(f.wert));
        setzeMitglied(o, "beleg", jsonTextOderNull(f.beleg));
        setzeMitglied(o, "beitrag", jsonZahlOderNull(f.beitrag));
        return objekt(o);
      }),
    ),
  );
  setzeMitglied(
    wurzel,
    "aufgaben",
    liste(
      e.aufgaben.map((a) => {
        const o: JsonMitglied[] = [];
        setzeMitglied(o, "text", jsonText(a.text));
        setzeMitglied(o, "frist", jsonTextOderNull(a.frist));
        setzeMitglied(o, "frist_text", jsonText(a.fristText));
        setzeMitglied(o, "beleg", jsonText(a.beleg));
        setzeMitglied(o, "beitrag", jsonGanzzahl(a.beitrag));
        return objekt(o);
      }),
    ),
  );
  setzeMitglied(wurzel, "offene_fragen", liste(e.offeneFragen.map(jsonText)));
  return objekt(wurzel);
}

/** Der kanonische Text (sortierte Schlüssel, 2 Leerzeichen, `\n` am Ende), bytegleich zur Swift-Seite. */
export function kodiereAuswertung(e: AuswertungErgebnis): string {
  return kodiereJson(auswertungBaum(e));
}

export type AuswertungLesen =
  | { ok: true; ergebnis: AuswertungErgebnis }
  | { ok: false; fehler: ProfilFehler[] };

const ISO_ZEIT = /^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$/;

function schemaCode(s: string): FehlerCode {
  const vorsatz = "lva.auswertung/";
  if (!s.startsWith(vorsatz)) return "schema_unbekannt";
  const rest = s.slice(vorsatz.length);
  if (!/^[1-9][0-9]*$/.test(rest)) return "schema_unbekannt";
  return rest === "1" ? "schema_unbekannt" : "schema_neuer";
}

/** Gültiger Kalendertag oder Uhrzeit im Muster `JJJJ-MM-TTThh:mm:ssZ`. */
function istIsoZeit(s: string): boolean {
  if (!ISO_ZEIT.test(s)) return false;
  if (parseIsoDatum(s.slice(0, 10)) === null) return false;
  return (
    Number(s.slice(11, 13)) <= 23 &&
    Number(s.slice(14, 16)) <= 59 &&
    Number(s.slice(17, 19)) <= 59
  );
}

class LeseFehler {
  fehler: ProfilFehler[] = [];
  melde(pfad: string, code: FehlerCode): void {
    this.fehler.push({ pfad, code });
  }
  text(
    o: JsonMitglied[],
    schluessel: string,
    pfad: string,
    min: number,
    max: number,
  ): string | null {
    const w = mitglied(o, schluessel);
    if (w === undefined) {
      this.melde(pfad, "fehlt");
      return null;
    }
    if (w.art !== "text") {
      this.melde(pfad, "typ");
      return null;
    }
    const n = zaehle(w.wert);
    if (n < min || n > max) this.melde(pfad, "laenge");
    return w.wert;
  }
  /** `null` ist erlaubt; liefert `undefined` bei Fehler, `null` bei null. */
  textOderNull(
    o: JsonMitglied[],
    schluessel: string,
    pfad: string,
    max: number,
  ): string | null | undefined {
    const w = mitglied(o, schluessel);
    if (w === undefined) {
      this.melde(pfad, "fehlt");
      return undefined;
    }
    if (w.art === "null") return null;
    if (w.art !== "text") {
      this.melde(pfad, "typ");
      return undefined;
    }
    if (zaehle(w.wert) > max) this.melde(pfad, "laenge");
    return w.wert;
  }
  /** Ganzzahl ab 1; `required` meldet sonst `fehlt`. */
  beitrag(
    o: JsonMitglied[],
    pfad: string,
    nullErlaubt: boolean,
  ): number | null | undefined {
    const w = mitglied(o, "beitrag");
    if (w === undefined) {
      this.melde(pfad, "fehlt");
      return undefined;
    }
    if (w.art === "null" && nullErlaubt) return null;
    if (w.art !== "zahl" || !/^(0|-?[1-9][0-9]*)$/.test(w.roh)) {
      this.melde(pfad, "typ");
      return undefined;
    }
    if (w.roh.length > 9 || Number(w.roh) < 1) {
      this.melde(pfad, "wert");
      return undefined;
    }
    return Number(w.roh);
  }
  liste(o: JsonMitglied[], schluessel: string, max: number): JsonWert[] {
    const w = mitglied(o, schluessel);
    if (w === undefined) {
      this.melde(schluessel, "fehlt");
      return [];
    }
    if (w.art !== "liste") {
      this.melde(schluessel, "typ");
      return [];
    }
    if (w.wert.length > max) this.melde(schluessel, "zu_viele");
    return w.wert;
  }
}

function leseFeld(
  l: LeseFehler,
  e: JsonWert,
  i: number,
  namen: Set<string>,
): FeldErgebnis | null {
  const pfad = `felder[${i}]`;
  if (e.art !== "objekt") {
    l.melde(pfad, "typ");
    return null;
  }
  const o = e.wert;
  const name = l.text(o, "name", `${pfad}.name`, 1, 40) ?? "";
  if (name !== "") {
    if (!istSchluessel(name)) l.melde(`${pfad}.name`, "muster");
    else if (namen.has(name)) l.melde(`${pfad}.name`, "doppelt");
    namen.add(name);
  }
  const typText = l.text(o, "typ", `${pfad}.typ`, 1, 40);
  const typ = (FELD_TYPEN as readonly string[]).includes(typText ?? "")
    ? (typText as FeldTyp)
    : "text";
  if (typText !== null && typ !== typText) l.melde(`${pfad}.typ`, "wert");
  const statusText = l.text(o, "status", `${pfad}.status`, 1, 40);
  const status = (FELD_STATI as readonly string[]).includes(statusText ?? "")
    ? (statusText as FeldStatus)
    : "nicht_genannt";
  if (statusText !== null && status !== statusText)
    l.melde(`${pfad}.status`, "wert");
  const wert = l.textOderNull(o, "wert", `${pfad}.wert`, MAX_WERT) ?? null;
  const beleg = l.textOderNull(o, "beleg", `${pfad}.beleg`, MAX_BELEG) ?? null;
  const beitrag = l.beitrag(o, `${pfad}.beitrag`, true) ?? null;
  if (
    status === "nicht_genannt" &&
    (wert !== null || beleg !== null || beitrag !== null)
  ) {
    l.melde(pfad, "wert");
  }
  if (status === "klar" && wert === null) l.melde(`${pfad}.wert`, "wert");
  return { name, typ, status, wert, beleg, beitrag };
}

function leseAufgabe(
  l: LeseFehler,
  e: JsonWert,
  i: number,
): AufgabeErgebnis | null {
  const pfad = `aufgaben[${i}]`;
  if (e.art !== "objekt") {
    l.melde(pfad, "typ");
    return null;
  }
  const o = e.wert;
  const text = l.text(o, "text", `${pfad}.text`, 1, MAX_AUFGABE_TEXT) ?? "";
  const fristWert = l.textOderNull(o, "frist", `${pfad}.frist`, 10);
  if (typeof fristWert === "string" && parseIsoDatum(fristWert) === null) {
    l.melde(`${pfad}.frist`, "muster");
  }
  const fristText =
    l.text(o, "frist_text", `${pfad}.frist_text`, 0, MAX_FRIST_TEXT) ?? "";
  const beleg = l.text(o, "beleg", `${pfad}.beleg`, 1, MAX_BELEG) ?? "";
  const beitrag = l.beitrag(o, `${pfad}.beitrag`, false) ?? 1;
  return { text, frist: fristWert ?? null, fristText, beleg, beitrag };
}

/** Liest und prüft ein Ergebnis `lva.auswertung/1`. Unbekannte Schlüssel werden verworfen; Fehler wie beim Profil (Pfad, Code). */
export function parseAuswertung(text: string): AuswertungLesen {
  const baum = parseJson(text);
  if (baum === null || baum.art !== "objekt") {
    return { ok: false, fehler: [{ pfad: "", code: "json" }] };
  }
  const o = baum.wert;
  const l = new LeseFehler();
  const schema = mitglied(o, "schema");
  if (schema === undefined) l.melde("schema", "fehlt");
  else if (schema.art !== "text") l.melde("schema", "typ");
  else if (schema.wert !== AUSWERTUNG_SCHEMA) {
    return {
      ok: false,
      fehler: [{ pfad: "schema", code: schemaCode(schema.wert) }],
    };
  }
  const assistent = l.text(o, "assistent", "assistent", 1, 64) ?? "";
  if (assistent !== "" && !istProfilId(assistent))
    l.melde("assistent", "muster");
  const erstellt = l.text(o, "erstellt", "erstellt", 0, 20) ?? "";
  if (erstellt !== "" && !istIsoZeit(erstellt)) l.melde("erstellt", "muster");
  const gekuerzt = mitglied(o, "gekuerzt");
  if (gekuerzt === undefined) l.melde("gekuerzt", "fehlt");
  else if (gekuerzt.art !== "bool") l.melde("gekuerzt", "typ");
  const zusammenfassung =
    l.text(o, "zusammenfassung", "zusammenfassung", 0, MAX_ZUSAMMENFASSUNG) ??
    "";

  const namen = new Set<string>();
  const felder: FeldErgebnis[] = [];
  l.liste(o, "felder", 30).forEach((e, i) => {
    const f = leseFeld(l, e, i, namen);
    if (f) felder.push(f);
  });
  const aufgaben: AufgabeErgebnis[] = [];
  l.liste(o, "aufgaben", MAX_AUFGABEN).forEach((e, i) => {
    const a = leseAufgabe(l, e, i);
    if (a) aufgaben.push(a);
  });
  const offeneFragen: string[] = [];
  l.liste(o, "offene_fragen", MAX_FRAGEN).forEach((e, i) => {
    const pfad = `offene_fragen[${i}]`;
    if (e.art !== "text") {
      l.melde(pfad, "typ");
      return;
    }
    if (zaehle(e.wert) > MAX_FRAGE) l.melde(pfad, "laenge");
    offeneFragen.push(e.wert);
  });

  if (l.fehler.length > 0) return { ok: false, fehler: ordneFehler(l.fehler) };
  return {
    ok: true,
    ergebnis: {
      assistent,
      erstellt,
      gekuerzt:
        gekuerzt !== undefined && gekuerzt.art === "bool" && gekuerzt.wert,
      zusammenfassung,
      felder,
      aufgaben,
      offeneFragen,
    },
  };
}
