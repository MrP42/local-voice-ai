import type { Sprache } from "./assistentPlan";

/**
 * Ergänzen und Korrigieren im Sprachgespräch (lva#485): reine Entscheidungslogik ohne Audio und
 * ohne Zustand. Die Engine ruft sie auf, wenn der Nutzer spricht, während die KI antwortet; sie
 * hält die Wiedergabe dann an (statt sie hart abzubrechen), lässt die Äußerung erkennen und
 * entscheidet erst mit dem Text.
 */

/** Rückmeldelaute (Deutsch und Englisch; auch mehrere Wörter): kein Beitrag, die Antwort läuft weiter. */
const RUECKMELDUNGEN: readonly string[] = [
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
  "yeah",
  "yes",
  "yep",
  "right",
  "sure",
  "uh huh",
  "got it",
  "i see",
];

/** Stoppwörter: die Antwort hält an, nichts geht ans Modell. */
const STOPP_KERN: readonly string[] = [
  "stopp",
  "stop",
  "halt",
  "warte",
  "moment",
  "wait",
  "hold on",
  "pause",
];
const STOPP_FUELL: readonly string[] = [
  "mal",
  "kurz",
  "bitte",
  "einen",
  "ein",
  "eine",
  "okay",
  "ja",
  "please",
  "just",
];

/** Längste Kette aus Rückmeldelauten („ja genau okay“), die noch als Rückmeldung gilt. */
const MAX_RUECKMELDUNG = 3;
const MAX_STOPP = 4;

/** Text für den Vergleich: klein, ohne Akzente und Satzzeichen, Wörter mit einem Leerzeichen. */
export function falte(text: string): string {
  return text
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^\p{L}\p{N}\s]+/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * Lässt sich der gefaltete Text lückenlos in höchstens `max` Einträge der Liste zerlegen
 * (Einträge dürfen aus mehreren Wörtern bestehen)? `kern`: mindestens einer davon muss aus dieser Liste sein.
 */
function zerlegbar(
  text: string,
  liste: readonly string[],
  max: number,
  kern?: readonly string[],
): boolean {
  const w = text === "" ? [] : text.split(" ");
  if (w.length === 0) return false;
  const geh = (i: number, n: number, hatKern: boolean): boolean => {
    if (i === w.length) return hatKern;
    if (n === max) return false;
    return liste.some((e) => {
      const ew = e.split(" ");
      if (ew.some((x, k) => w[i + k] !== x)) return false;
      return geh(i + ew.length, n + 1, hatKern || !!kern?.includes(e));
    });
  };
  return geh(0, 0, kern === undefined);
}

/** Besteht die Äußerung nur aus Rückmeldelauten („mhm“, „ja“, „okay“, „yeah“ …)? */
export function istRueckmeldung(text: string): boolean {
  return zerlegbar(falte(text), RUECKMELDUNGEN, MAX_RUECKMELDUNG);
}

/** Besteht die Äußerung nur aus einem Stoppwort („stopp“, „halt“, „warte“, „moment“, „stop“, „wait“ …) samt Füllwörtern? */
export function istStoppwort(text: string): boolean {
  return zerlegbar(
    falte(text),
    [...STOPP_KERN, ...STOPP_FUELL],
    MAX_STOPP,
    STOPP_KERN,
  );
}

/** Endet der Satz auf eine Frage (auch mit Anführungszeichen oder Klammer dahinter)? */
export function istFrage(satz: string): boolean {
  return /\?["'“”„»«)\]\s]*$/.test(satz.trim());
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
 * der die laufende Antwort gehört; `satz` der Satz der Antwort, der gerade läuft oder zuletzt
 * gesprochen wurde: war er eine Frage, ist „ja“ darauf eine Antwort und kein Rückmeldelaut.
 */
export function entscheide(
  text: string,
  stand: TurnStand | null,
  vorherig = "",
  satz = "",
): Entscheidung {
  if (stand === null) return { art: "neu", text, abbrechen: false };
  const frage = istFrage(satz);
  if (!frage && istRueckmeldung(text)) return { art: "weiter" };
  if (istStoppwort(text)) return { art: "stopp" };
  // Die Antwort auf eine Frage gehört nicht in die Äußerung, auf die die Frage folgte.
  if (!frage && darfZusammenfuehren(stand))
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
