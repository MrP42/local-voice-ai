import type { Sprache } from "./assistentPlan";

/**
 * Ergänzen und Korrigieren im Sprachgespräch (lva#485): reine Entscheidungslogik ohne Audio und
 * ohne Zustand. Die Engine ruft sie auf, wenn der Nutzer spricht, während die KI antwortet; sie
 * hält die Wiedergabe dann an (statt sie hart abzubrechen), lässt die Äußerung erkennen und
 * entscheidet erst mit dem Text.
 */

/** Rückmeldelaute: kein Beitrag, die Antwort läuft weiter. */
const RUECKMELDUNGEN = new Set([
  "mhm",
  "mhmm",
  "mmhm",
  "mmh",
  "mm",
  "hm",
  "hmm",
  "hmmm",
  "ja",
  "okay",
  "ok",
  "aha",
  "genau",
  "stimmt",
  "verstehe",
]);

/** Stoppwörter: die Antwort hält an, nichts geht ans Modell. */
const STOPP_KERN = new Set(["stopp", "stop", "halt", "warte", "moment"]);
const STOPP_FUELL = new Set(["mal", "kurz", "bitte", "einen", "ein", "eine"]);

/** Längste Kette aus Rückmeldelauten („ja genau okay“), die noch als Rückmeldung gilt. */
const MAX_RUECKMELDUNG = 3;
const MAX_STOPP = 4;

/** Text für den Vergleich: klein, ohne Akzente und Satzzeichen, Wörter mit einem Leerzeichen. */
export function falte(text: string): string {
  return text
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^\p{L}\p{N}\s]+/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function woerter(text: string): string[] {
  const f = falte(text);
  return f === "" ? [] : f.split(" ");
}

/** Besteht die Äußerung nur aus Rückmeldelauten („mhm“, „ja“, „okay“ …)? */
export function istRueckmeldung(text: string): boolean {
  const w = woerter(text);
  return (
    w.length > 0 &&
    w.length <= MAX_RUECKMELDUNG &&
    w.every((x) => RUECKMELDUNGEN.has(x))
  );
}

/** Besteht die Äußerung nur aus einem Stoppwort („stopp“, „halt“, „warte“, „moment“)? */
export function istStoppwort(text: string): boolean {
  const w = woerter(text);
  return (
    w.length > 0 &&
    w.length <= MAX_STOPP &&
    w.some((x) => STOPP_KERN.has(x)) &&
    w.every((x) => STOPP_KERN.has(x) || STOPP_FUELL.has(x))
  );
}

/** Was die Engine vom laufenden Schritt weiß. */
export interface TurnStand {
  /** Der Schritt entstand aus gesprochener Eingabe (nicht getippt). */
  ausSprache: boolean;
  /** Vorlage, S0-Antwort oder Bedienung: wirkt schon, lässt sich nicht neu stellen. */
  vorlage: boolean;
  /** Der erste Satz der Modellantwort ist zu Ende gesprochen. */
  ersterSatzFertig: boolean;
  /** Der Beitrag steht schon im gespeicherten Gespräch (ohne Rust nicht mehr zu ersetzen). */
  schonGespeichert: boolean;
}

/** Darf die neue Äußerung mit der vorherigen zu einer zusammengeführt werden? */
export function darfZusammenfuehren(s: TurnStand): boolean {
  return (
    s.ausSprache && !s.vorlage && !s.ersterSatzFertig && !s.schonGespeichert
  );
}

/** Zwei Äußerungen zu einer: `<alt> <neu>`. */
export function fuegeZusammen(alt: string, neu: string): string {
  return [alt.trim(), neu.trim()].filter((t) => t !== "").join(" ");
}

export type Entscheidung =
  /** Rückmeldelaut: die Wiedergabe läuft weiter, es gibt keinen Beitrag. */
  | { art: "weiter" }
  /** Stoppwort: die Antwort hält an, es gibt keinen Beitrag. */
  | { art: "stopp" }
  /** Ergänzung: die Antwort wird still verworfen, `text` ist die eine Äußerung. */
  | { art: "zusammenfuehren"; text: string }
  /** Neuer Beitrag; `abbrechen`: eine noch laufende Antwort wird dafür unterbrochen. */
  | { art: "neu"; text: string; abbrechen: boolean };

/**
 * Entscheidung, sobald der Text einer Äußerung vorliegt, die während der Antwort begann.
 * `stand` ist `null`, wenn keine Antwort mehr läuft (sie ist zwischenzeitlich zu Ende gegangen):
 * dann ist jede Äußerung ein gewöhnlicher neuer Beitrag. `vorherig` ist die Nutzeräußerung, zu
 * der die laufende Antwort gehört.
 */
export function entscheide(
  text: string,
  stand: TurnStand | null,
  vorherig = "",
): Entscheidung {
  if (stand === null) return { art: "neu", text, abbrechen: false };
  if (istRueckmeldung(text)) return { art: "weiter" };
  if (istStoppwort(text)) return { art: "stopp" };
  if (darfZusammenfuehren(stand))
    return { art: "zusammenfuehren", text: fuegeZusammen(vorherig, text) };
  return { art: "neu", text, abbrechen: true };
}

const HINWEIS: Record<Sprache, string> = {
  de: "Hinweis: Deine letzte Antwort wurde vom Gesprächspartner unterbrochen; er hat nur den Anfang gehört. Gehe auf seine neue Äußerung ein und wiederhole nicht, was du schon gesagt hast, außer er fragt danach.",
  en: "Note: The other person interrupted your last answer and only heard its beginning. Respond to what they say now and do not repeat what you already said unless they ask for it.",
};

/** Kurzer Systemhinweis für den nächsten Aufruf, wenn die letzte Antwort unterbrochen wurde. */
export function unterbrechungsHinweis(
  unterbrochen: boolean,
  sprache: Sprache,
): string | null {
  return unterbrochen ? HINWEIS[sprache] : null;
}

/** Die Systemanweisung, bei Bedarf um den Unterbrechungshinweis ergänzt. */
export function systemMitHinweis(
  system: string,
  unterbrochen: boolean,
  sprache: Sprache,
): string {
  const hinweis = unterbrechungsHinweis(unterbrochen, sprache);
  return hinweis ? `${system}\n\n${hinweis}` : system;
}
