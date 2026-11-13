/**
 * Was ein gewählter Assistent im Sprachgespräch bewirkt (lva#556, Paket G2). Reine Funktionen ohne Tauri und ohne Store,
 * damit Engine, Orchestrierung und Oberfläche dasselbe benutzen und Node-Tests sie laden können. Der Kern
 * (`../assistenten`) wird nur benutzt, nie geändert.
 *
 * „Alles“ ist kein Eingriff: `wirkungFuer` liefert dafür `null`, und jede Funktion hier behandelt `null` als „es gilt, was die
 * Nutzereinstellungen sagen“. So bleibt das Verhalten ohne Auswahl und mit „Alles“ gleich dem von vor dem Paket.
 */
import {
  ALLES_ID,
  loeseBegruessung,
  rollenblock,
  wirksameBereiche,
} from "../assistenten";
import type {
  PcBereich,
  PlatzhalterWerte,
  Profil,
  Rollenblock,
  Verhalten,
} from "../assistenten";
import { ALL_AREAS } from "../bereiche";
import { DEFAULT_TURN_CONFIG, type TurnConfig } from "../turnTaking";

/** Ein wirksamer Assistent (nicht „Alles“): das Profil und die Werte der Platzhalter beim Start des Gesprächs. */
export interface AssistentWirkung {
  profil: Profil;
  /** Liefert die Platzhalterwerte (Datum, Uhrzeit, Name); die Engine ruft es beim Start eines Gesprächs. */
  werte: () => PlatzhalterWerte;
}

/** „Alles“ wirkt nicht (keine Anweisungen, keine Begrenzung, keine eigene Begrüßung); jeder andere Assistent schon. */
export function wirkungFuer(
  profil: Profil,
  werte: () => PlatzhalterWerte,
): AssistentWirkung | null {
  return profil.id === ALLES_ID ? null : { profil, werte };
}

/** Kalenderfelder der Ortszeit für die Platzhalter; der Name fehlt, wenn keiner da ist (nie ein Ersatzname). */
export function werteJetzt(
  name: string | null,
  jetzt: Date = new Date(),
): PlatzhalterWerte {
  return {
    name: name && name.trim() ? name.trim() : null,
    jahr: jetzt.getFullYear(),
    monat: jetzt.getMonth() + 1,
    tag: jetzt.getDate(),
    wochentag: ((jetzt.getDay() + 6) % 7) + 1,
    stunde: jetzt.getHours(),
    minute: jetzt.getMinutes(),
  };
}

/** Der Stand des Assistenten an der Engine (Spiegel von `AssistentStand` der Engine, ohne sie zu laden). */
export interface GiltStand {
  /** Mit diesem Assistenten läuft das offene Gespräch; `null`: keiner. */
  gilt: AssistentWirkung | null;
  /** Die Auswahl ist eine andere und gilt erst ab dem nächsten Gespräch. */
  ausstehend: boolean;
}

/**
 * Welcher Assistent im offenen Gespräch gilt, für die Anzeige (Begrüßung, Bereichs-Chip): im Projektgespräch keiner; sonst der
 * gewählte, außer die Auswahl ist erst nach dem Beginn des Gesprächs eingetroffen (`ausstehend`): dann gilt noch der, mit dem
 * das Gespräch läuft.
 */
export function geltender(
  imProjekt: boolean,
  stand: GiltStand,
  gewaehlt: AssistentWirkung | null,
): { wirkung: AssistentWirkung | null; ausstehend: boolean } {
  if (imProjekt) return { wirkung: null, ausstehend: false };
  return stand.ausstehend
    ? { wirkung: stand.gilt, ausstehend: true }
    : { wirkung: gewaehlt, ausstehend: false };
}

// ---------------------------------------------------------------- Rollenblock

/** Zeichen, die ein Modell mit höchstens 4096 Token Kontext für die Rolle bekommt. */
export const ROLLE_BUDGET_KLEIN = 1200;
/** Obergrenze: so viel erlaubt das Profil an Anweisungen höchstens (8000 Zeichen). */
export const ROLLE_BUDGET_MAX = 8000;
/** Kontext eines lokalen Modells, wenn die Einstellungen keinen nennen (Standard des Kerns in Rust). */
export const KONTEXT_LOKAL_STANDARD = 8192;

/** Ein Modell, soweit es das Budget betrifft. */
export interface BudgetModell {
  /** Größe des Kontexts in Token (`max_input_tokens`, sonst `context_limit` der Einstellungen); unbekannt: `null`. */
  contextTokens?: number | null;
  local: boolean;
}

/**
 * Budget in Zeichen für den Rollenblock: bis 4096 Token Kontext 1200 Zeichen, sonst höchstens 8000 und höchstens drei Fünftel
 * des Kontexts. Ist die Größe unbekannt, gilt bei einem lokalen Modell der Standard (8192), bei einem entfernten die Obergrenze.
 */
export function rollenBudget(modell: BudgetModell): number {
  const roh = modell.contextTokens;
  const bekannt = typeof roh === "number" && Number.isFinite(roh) && roh > 0;
  const tokens = bekannt
    ? roh
    : modell.local
      ? KONTEXT_LOKAL_STANDARD
      : Number.POSITIVE_INFINITY;
  if (tokens <= 4096) return ROLLE_BUDGET_KLEIN;
  return Math.min(ROLLE_BUDGET_MAX, Math.floor((tokens * 3) / 5));
}

/** Das kleinste Budget der beteiligten Modelle (Sprecher und Denker bekommen denselben Block); ohne Modell der lokale Standard. */
export function kleinstesBudget(modelle: readonly BudgetModell[]): number {
  if (modelle.length === 0)
    return rollenBudget({ contextTokens: null, local: true });
  return Math.min(...modelle.map(rollenBudget));
}

/** Der Rollenblock des Assistenten für höchstens `budgetZeichen` Zeichen (leer ohne Anweisungen). */
export function rolle(
  w: AssistentWirkung,
  werte: PlatzhalterWerte,
  budgetZeichen: number,
): Rollenblock {
  return rollenblock(w.profil, werte, budgetZeichen);
}

/** Der Rollenblock steht VOR den Regeln („Diese Einstellung ändert keine der folgenden Regeln“). */
export function mitRolle(rollentext: string, regeln: string): string {
  return rollentext ? `${rollentext}\n\n${regeln}` : regeln;
}

// ---------------------------------------------------------------- Verhalten

/** Stille bis zum spekulativen Start und bis zum sicheren Satzende je Sprechpause (ms); `normal` ist die Vorgabe der App. */
const PAUSEN: Readonly<
  Record<Verhalten["sprechpause"], { earlyEndMs: number; endMs: number }>
> = {
  kurz: { earlyEndMs: 300, endMs: 500 },
  normal: {
    earlyEndMs: DEFAULT_TURN_CONFIG.earlyEndMs,
    endMs: DEFAULT_TURN_CONFIG.endMs,
  },
  lang: { earlyEndMs: 700, endMs: 1300 },
};

/**
 * Die Einstellungen der Äußerungserkennung: mit „Alles“ (`null`) genau die des Nutzers, sonst Sprechpause und Unterbrechen aus
 * dem Profil; Pegel und Schwellen bleiben die des Nutzers. Die Werte liegen im Bereich der Regler der Gesprächseinstellungen.
 */
export function wirksamerTurn(
  nutzer: TurnConfig,
  w: AssistentWirkung | null,
): TurnConfig {
  if (!w) return nutzer;
  const v = w.profil.verhalten;
  return { ...nutzer, ...PAUSEN[v.sprechpause], bargeIn: v.unterbrechen };
}

// ---------------------------------------------------------------- Wissen

/** Bereiche mit Abrufindex (Rust: `kind_of`); Termine und Notizbuch sind nur Werkzeuge. */
const ABRUF_BEREICHE: readonly string[] = [
  "meetings",
  "dictations",
  "conversations",
  "tts",
  "books",
  "book_notes",
  "ki_docs",
];

/**
 * Die Obergrenze, die der Assistent den Bereichen setzt (Schlüssel des PC), oder `null`, wenn er alle Bereiche will. Sie geht
 * als Begrenzung an Rust (Werkzeuge und Abruf); die Freigabe der Einstellungen gilt dort weiter, die Begrenzung öffnet nie mehr.
 */
export function begrenzung(w: AssistentWirkung | null): PcBereich[] | null {
  if (!w || w.profil.wissen.alleBereiche) return null;
  return wirksameBereiche(w.profil, ALL_AREAS);
}

/** Die wirksamen Bereiche: die vom Assistenten gewollten, geschnitten mit den freigegebenen. */
export function wirksam(
  w: AssistentWirkung | null,
  freigegeben: readonly string[],
): string[] {
  return w ? wirksameBereiche(w.profil, freigegeben) : [...freigegeben];
}

/** Bleibt unter der Begrenzung ein Bereich mit Abrufindex? Sonst gibt es keinen Abruf für das Gespräch. */
export function hatAbrufBereich(grenze: readonly string[] | null): boolean {
  return grenze === null || grenze.some((b) => ABRUF_BEREICHE.includes(b));
}

// ---------------------------------------------------------------- Begrüßung

/**
 * Die feste Begrüßung des Assistenten (kein Modell, kein Verlaufseintrag), Platzhalter aufgelöst; ohne Namen entfällt die
 * Anrede (das macht der Kern). `null` mit „Alles“ und bei leerer Begrüßung: dann gilt die allgemeine Begrüßung der Startseite.
 */
export function begruessung(
  w: AssistentWirkung | null,
  werte: PlatzhalterWerte,
): string | null {
  if (!w) return null;
  const text = loeseBegruessung(w.profil, werte).text;
  return text === "" ? null : text;
}
