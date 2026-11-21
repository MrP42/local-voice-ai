/**
 * Die Logik hinter dem Editor eigener Assistenten (lva#556, G2 Teil 2): Entwurf, Zähler gegen das Budget der Rolle, Fehler je
 * Feld, Bereichsauswahl und der Abgleich des Entwurfs mit einer inzwischen geänderten Datei. Reine Funktionen ohne Tauri und
 * ohne Oberfläche, damit Node-Tests sie laden können.
 *
 * Der Entwurf ist ein vollständiges `Profil` (mit seinen `extra`-Feldern): gespeichert wird `kodiereProfil` des Entwurfs, so
 * bleiben unbekannte Felder und alles, was der Editor nicht anzeigt (Kontexte, Schnittstellen, Auswertung), unverändert. Es
 * wird nie still repariert: eine Eingabe außerhalb der Grenzen bleibt stehen, das Feld meldet den Fehler des Kerns.
 */
import {
  AUSSPRACHE_MAX,
  AUSSPRACHE_TEXT_MAX,
  BEKANNTE_BEREICHE,
  kodiereProfil,
  loesePlatzhalter,
  neuesProfil,
  pruefeProfil,
  wirksameBereiche,
} from "../assistenten";
import type {
  AusspracheEintrag,
  FehlerCode,
  PcBereich,
  PlatzhalterWerte,
  Profil,
  ProfilFehler,
} from "../assistenten";
import { ROLLE_BUDGET_KLEIN, ROLLE_BUDGET_MAX } from "./wirkung";
import { ALL_AREAS } from "../bereiche";

/** Grenzen der Felder in Unicode-Zeichen (Kern). */
export const GRENZEN = {
  name: 60,
  beschreibung: 200,
  anweisungen: ROLLE_BUDGET_MAX,
  begruessung: 300,
  stimme: 120,
  /** Ausspracheregeln je Profil. */
  aussprache: AUSSPRACHE_MAX,
  /** Zeichen von `wort` und von `aussprache` einer Regel. */
  ausspracheText: AUSSPRACHE_TEXT_MAX,
} as const;

/** Wie viele Sekunden Stille bis zur Nachfrage zur Wahl stehen (0 = aus; der Kern erlaubt 0 und 5 bis 120). */
export const NACHFRAGE_STUFEN: readonly number[] = [
  0, 5, 10, 15, 20, 30, 45, 60, 90, 120,
];
/** Die Sekunden, die das Einschalten der Nachfrage vorschlägt. */
export const NACHFRAGE_VORSCHLAG = 15;

/** Die Sprachen, die der Editor anbietet (Schlüssel des Profils: `auto` oder Sprachcode). */
export const SPRACHEN: readonly string[] = [
  "de",
  "en",
  "fr",
  "es",
  "it",
  "nl",
  "pl",
  "pt",
  "tr",
  "sv",
  "da",
  "cs",
  "uk",
  "ru",
  "auto",
];

/** Anzahl der Unicode-Zeichen (so zählt der Kern). */
export const zeichen = (s: string): number => Array.from(s).length;

// ---------------------------------------------------------------- Fehler je Feld

export const FELDER = [
  "name",
  "beschreibung",
  "anweisungen",
  "begruessung",
  "sprache",
  "stimme",
  "verhalten",
  "wissen",
  "aussprache",
  "sonst",
] as const;
export type FeldId = (typeof FELDER)[number];

/** Das Feld des Editors, zu dem ein Pfad der Prüfung gehört (alles Unbekannte unter `sonst`). */
export function feldVon(pfad: string): FeldId {
  const kopf = pfad.split(/[.[]/, 1)[0];
  return (FELDER as readonly string[]).includes(kopf) && kopf !== "sonst"
    ? (kopf as FeldId)
    : "sonst";
}

export type FehlerJeFeld = Partial<Record<FeldId, ProfilFehler[]>>;

/** Die Fehler der Prüfung, nach Feld geordnet (nur Felder mit Fehlern kommen vor). */
export function fehlerJeFeld(fehler: readonly ProfilFehler[]): FehlerJeFeld {
  const aus: FehlerJeFeld = {};
  for (const f of fehler) (aus[feldVon(f.pfad)] ??= []).push(f);
  return aus;
}

/** Prüft den Entwurf mit dem Kern; leer heißt: speicherbar. */
export const pruefe = (entwurf: Profil): ProfilFehler[] =>
  pruefeProfil(entwurf);

// ---------------------------------------------------------------- Zähler der Anweisungen

export interface RollenStand {
  /** Zeichen im Feld (gilt gegen die Obergrenze des Formats). */
  zeichen: number;
  /** Zeichen nach dem Auflösen der Platzhalter (das bekommt das Modell, vor dem Kürzen). */
  aufgeloest: number;
  /** Das Feld ist länger als das Format erlaubt (der Kern lehnt es ab). */
  ueberMax: boolean;
  /** Mehr, als ein kleines Modell (bis 4096 Token Kontext) bekommt: es sieht nur den Anfang. */
  ueberKlein: boolean;
  /** Platzhalter, die es nicht gibt (sie entfallen im Gespräch samt Satzzeichen davor). */
  unbekannt: string[];
}

/** Der Stand der Anweisungen gegen das Budget (`ROLLE_BUDGET_KLEIN` für kleine, `ROLLE_BUDGET_MAX` für alle Modelle). */
export function rollenStand(p: Profil, werte: PlatzhalterWerte): RollenStand {
  const roh = zeichen(p.anweisungen);
  const geloest = loesePlatzhalter(p.anweisungen, p.sprache, werte);
  const aufgeloest = zeichen(geloest.text.trim());
  return {
    zeichen: roh,
    aufgeloest,
    ueberMax: roh > ROLLE_BUDGET_MAX,
    ueberKlein: aufgeloest > ROLLE_BUDGET_KLEIN,
    unbekannt: geloest.unbekannt,
  };
}

/** Die Platzhalter, die der Editor als Einfügehilfe anbietet (`{{name}}` und die Kalenderwerte). */
export const PLATZHALTER: readonly string[] = [
  "name",
  "tageszeit",
  "datum",
  "wochentag",
  "uhrzeit",
];

// ---------------------------------------------------------------- Änderungen am Entwurf

/** Flache Kopie mit geänderten obersten Feldern; `verhalten` und `wissen` werden eigens kopiert (kein Teilen mit der Quelle). */
export function mit(p: Profil, aenderung: Partial<Omit<Profil, "id">>): Profil {
  return { ...p, ...aenderung };
}

export function mitVerhalten(
  p: Profil,
  aenderung: Partial<Profil["verhalten"]>,
): Profil {
  return { ...p, verhalten: { ...p.verhalten, ...aenderung } };
}

export function mitWissen(
  p: Profil,
  aenderung: Partial<Profil["wissen"]>,
): Profil {
  return { ...p, wissen: { ...p.wissen, ...aenderung } };
}

/**
 * Schaltet einen Bereich des Profils ein oder aus. Die Reihenfolge folgt den bekannten Bereichen; Schlüssel, die der Editor
 * nicht kennt (etwa von einer neueren Fassung), bleiben stehen.
 */
export function bereichSchalten(
  p: Profil,
  schluessel: string,
  an: boolean,
): Profil {
  const aktuell = new Set(p.wissen.bereiche);
  if (an) aktuell.add(schluessel);
  else aktuell.delete(schluessel);
  const bekannt = (BEKANNTE_BEREICHE as readonly string[]).filter((b) =>
    aktuell.has(b),
  );
  const fremd = p.wissen.bereiche.filter(
    (b) =>
      aktuell.has(b) && !(BEKANNTE_BEREICHE as readonly string[]).includes(b),
  );
  return mitWissen(p, { bereiche: bekannt.concat(fremd) });
}

/** Der Bereich des PC zu einem Schlüssel des Profils; `null` ohne Wirkung am PC (`ablage`) oder bei einem unbekannten. */
export function pcBereich(schluessel: string): PcBereich | null {
  const leer = neuesProfil("probe", "probe");
  const probe = { ...leer, wissen: { ...leer.wissen, bereiche: [schluessel] } };
  const [bereich] = wirksameBereiche(probe, ALL_AREAS);
  return bereich ?? null;
}

export interface BereichsAnzeige {
  schluessel: string;
  /** Der Bereich des PC; `null`: am PC ohne Wirkung. */
  pc: PcBereich | null;
  /** Der Assistent will ihn. */
  gewollt: boolean;
  /** In den Einstellungen freigegeben (nur dann kann der Assistent ihn öffnen). */
  freigegeben: boolean;
  /** Im Gespräch wirksam: gewollt und freigegeben. */
  wirksam: boolean;
}

/**
 * Wie sich die Auswahl des Profils zu den Freigaben der Einstellungen verhält: Das Profil begrenzt nur; was die Einstellungen
 * nicht freigeben, öffnet es nie. `freigegeben` sind die Bereiche des PC, die die Einstellungen freigeben.
 */
export function bereichsAnzeige(
  p: Profil,
  freigegeben: readonly string[],
): BereichsAnzeige[] {
  const wirksam = new Set<string>(wirksameBereiche(p, freigegeben));
  const alle = [
    ...(BEKANNTE_BEREICHE as readonly string[]),
    ...p.wissen.bereiche.filter(
      (b) => !(BEKANNTE_BEREICHE as readonly string[]).includes(b),
    ),
  ];
  return alle.map((schluessel) => {
    const pc = pcBereich(schluessel);
    return {
      schluessel,
      pc,
      gewollt: p.wissen.alleBereiche || p.wissen.bereiche.includes(schluessel),
      freigegeben: pc !== null && freigegeben.includes(pc),
      wirksam: pc !== null && wirksam.has(pc),
    };
  });
}

/** Wie viele Bereiche im Gespräch wirken (gewollt und freigegeben) und wie viele die Einstellungen freigeben. */
export function bereichsSumme(
  p: Profil,
  freigegeben: readonly string[],
): { wirksam: number; freigegeben: number } {
  return {
    wirksam: wirksameBereiche(p, freigegeben).length,
    freigegeben: ALL_AREAS.filter((b) => freigegeben.includes(b)).length,
  };
}

// ---------------------------------------------------------------- Aussprache

/** Die Regeln des Profils (fehlt das Feld, ist die Liste leer). */
export function ausspracheListe(p: Profil): readonly AusspracheEintrag[] {
  return p.aussprache ?? [];
}

/**
 * Setzt die Liste der Regeln. Eine leere Liste lässt das Feld **wegfallen**: es wird nie `[]` geschrieben (das Feld hat keinen
 * Standardwert, ein Profil ohne Regeln bleibt Byte für Byte, wie es vor dem Feld war).
 */
export function mitAussprache(
  p: Profil,
  liste: readonly AusspracheEintrag[],
): Profil {
  const neu: Profil = { ...p };
  if (liste.length === 0) delete neu.aussprache;
  else neu.aussprache = [...liste];
  return neu;
}

/** Hängt eine leere Regel an; ab `AUSSPRACHE_MAX` Regeln ändert sich nichts. Die neue Regel muss noch ausgefüllt werden. */
export function ausspracheHinzu(p: Profil): Profil {
  const liste = ausspracheListe(p);
  if (liste.length >= AUSSPRACHE_MAX) return p;
  return mitAussprache(p, [...liste, { wort: "", aussprache: "" }]);
}

/** Ändert `wort` oder `aussprache` der Regel an Stelle `i`; die unbekannten Felder der Regel (`extra`) bleiben. */
export function ausspracheAendern(
  p: Profil,
  i: number,
  feld: "wort" | "aussprache",
  wert: string,
): Profil {
  const liste = ausspracheListe(p);
  if (i < 0 || i >= liste.length) return p;
  return mitAussprache(
    p,
    liste.map((e, j) => (j === i ? { ...e, [feld]: wert } : e)),
  );
}

/** Entfernt die Regel an Stelle `i`; war es die letzte, fällt das Feld weg. */
export function ausspracheEntfernen(p: Profil, i: number): Profil {
  const liste = ausspracheListe(p);
  if (i < 0 || i >= liste.length) return p;
  return mitAussprache(
    p,
    liste.filter((_e, j) => j !== i),
  );
}

/** Verschiebt die Regel an Stelle `i` um eine Stelle (`-1` nach oben, `1` nach unten); am Rand ändert sich nichts. */
export function ausspracheVerschieben(
  p: Profil,
  i: number,
  richtung: -1 | 1,
): Profil {
  const liste = [...ausspracheListe(p)];
  const j = i + richtung;
  if (i < 0 || i >= liste.length || j < 0 || j >= liste.length) return p;
  [liste[i], liste[j]] = [liste[j], liste[i]];
  return mitAussprache(p, liste);
}

/** Die Befunde der Prüfung zu den Regeln: die der Liste als Ganzes und je Zeile nach Feld. */
export interface AusspracheBefunde {
  /** Zur Liste selbst (zu viele Regeln, falscher Typ). */
  liste: FehlerCode[];
  /** Je Stelle in der Liste. */
  zeilen: Record<number, ZeilenBefund>;
}
export interface ZeilenBefund {
  wort: FehlerCode[];
  aussprache: FehlerCode[];
  /** Zur Regel als Ganzem oder zu einem Feld, das der Editor nicht kennt. */
  zeile: FehlerCode[];
}

const AUSSPRACHE_PFAD = /^aussprache(?:\[(\d+)\](?:\.(.*))?)?$/;

/** Ordnet die Fehler zu `aussprache` (Pfade `aussprache`, `aussprache[i]`, `aussprache[i].wort`, …) der Liste und den Zeilen zu. */
export function ausspracheBefunde(
  fehler: readonly ProfilFehler[],
): AusspracheBefunde {
  const aus: AusspracheBefunde = { liste: [], zeilen: {} };
  for (const f of fehler) {
    const m = AUSSPRACHE_PFAD.exec(f.pfad);
    if (!m) continue;
    if (m[1] === undefined) {
      aus.liste.push(f.code);
      continue;
    }
    const zeile = (aus.zeilen[Number(m[1])] ??= {
      wort: [],
      aussprache: [],
      zeile: [],
    });
    if (m[2] === "wort") zeile.wort.push(f.code);
    else if (m[2] === "aussprache") zeile.aussprache.push(f.code);
    else zeile.zeile.push(f.code);
  }
  return aus;
}

/**
 * Das Profil, wie es geschrieben wird: eine leere Liste der Aussprache fällt weg (nie `[]`). Das gilt auch für ein Profil, das
 * eine leere Liste schon in der Datei hatte: es wird ohne sie geschrieben, sobald es etwas anderes zu speichern gibt.
 */
export function zuSchreiben(p: Profil): Profil {
  return p.aussprache && p.aussprache.length === 0 ? mitAussprache(p, []) : p;
}

// ---------------------------------------------------------------- Änderungen erkennen, Abgleich mit der Datei

/**
 * Der Entwurf unterscheidet sich vom gespeicherten Stand (verglichen wird die kanonische Ausgabe, so wie geschrieben würde:
 * eine leere Liste der Aussprache und ein fehlendes Feld sind dasselbe).
 */
export function istGeaendert(gespeichert: Profil, entwurf: Profil): boolean {
  return (
    kodiereProfil(zuSchreiben(gespeichert)) !==
    kodiereProfil(zuSchreiben(entwurf))
  );
}

export type Abgleich =
  /** Die Datei ist unverändert. */
  | { art: "gleich" }
  /** Die Datei hat sich geändert, der Entwurf nicht: der neue Stand gilt. */
  | { art: "uebernommen"; entwurf: Profil }
  /** Nur der Name hat sich geändert (Umbenennen in der Liste), die Änderungen im Entwurf bleiben. */
  | { art: "umbenannt"; entwurf: Profil }
  /** Die Datei hat sich geändert, während der Entwurf Änderungen hat: der Mensch entscheidet. */
  | { art: "konflikt" };

/**
 * Gleicht den Entwurf mit der Datei ab, wenn sich diese unter ihm geändert hat. `basis` ist der Stand, auf dem der Entwurf
 * begann, `jetzt` der aktuelle Stand der Datei. Steht in der Datei schon, was der Entwurf sagt, gilt die Datei. Ein reines
 * Umbenennen übernimmt der Entwurf (außer der Name ist im Entwurf selbst geändert); jede andere Änderung neben einem
 * geänderten Entwurf ist ein Konflikt und wird nie still überschrieben.
 */
export function gleicheAb(
  basis: { profil: Profil; text: string },
  jetzt: { profil: Profil; text: string },
  entwurf: Profil,
): Abgleich {
  if (basis.text === jetzt.text) return { art: "gleich" };
  // Die Datei hat den Stand des Entwurfs schon (etwa gleich nach dem eigenen Speichern): kein Konflikt, nur ein neuer Bezug.
  if (
    !istGeaendert(basis.profil, entwurf) ||
    !istGeaendert(jetzt.profil, entwurf)
  )
    return { art: "uebernommen", entwurf: jetzt.profil };
  const nurName =
    kodiereProfil({ ...basis.profil, name: jetzt.profil.name }) ===
    kodiereProfil(jetzt.profil);
  if (nurName)
    return {
      art: "umbenannt",
      entwurf:
        entwurf.name === basis.profil.name
          ? { ...entwurf, name: jetzt.profil.name }
          : entwurf,
    };
  return { art: "konflikt" };
}

/** Der Name, den „Als Kopie speichern“ vorschlägt (nach dem Konflikt oder beim Kopieren). */
export function kopieName(name: string, kopie: string): string {
  const roh = `${name} (${kopie})`;
  const z = Array.from(roh);
  if (z.length <= GRENZEN.name) return roh;
  const rest = Array.from(` (${kopie})`).length;
  return `${Array.from(name)
    .slice(0, GRENZEN.name - rest)
    .join("")
    .trimEnd()} (${kopie})`;
}
