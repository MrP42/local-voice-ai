// Texte, mit denen der Orchestrator (vendort) und die Buch-Seite eine Wartezeit überbrücken
// („Moment, ich schaue kurz nach.“). Reine Daten ohne Abhängigkeiten, damit die Engine sie prüfen
// kann, ohne den Orchestrator zu laden (Test: scripts/assistent-engine.test.mjs vergleicht die
// vendorten Konstanten mit dieser Liste).

/** Ankündigung je Werkzeug (lesende Werkzeuge der Agentenbrücke). */
export const ANNOUNCE: Record<string, string> = {
  meetings_list: "Moment, ich schaue in den Besprechungen nach.",
  meetings_search: "Moment, ich suche in den Besprechungen.",
  meeting_transcript: "Moment, ich lese im Transkript nach.",
  dictations_recent: "Moment, ich schaue in den Diktaten nach.",
  dictations_search: "Moment, ich suche in den Diktaten.",
  conversations_list: "Moment, ich schaue in den früheren Gesprächen nach.",
  conversations_search: "Moment, ich suche in den früheren Gesprächen.",
  conversation_get: "Moment, ich lese das frühere Gespräch.",
  calendar_events: "Moment, ich schaue in den Terminen nach.",
  day_overview: "Moment, ich verschaffe mir einen Überblick über den Tag.",
  tts_pages_list: "Moment, ich schaue in den Vorlesen-Seiten nach.",
  tts_page_text: "Moment, ich lese die Seite.",
};

/** Standard-Überbrückungen des vendorten Orchestrators (`DEFAULT_BRIDGE`, `DENKER_BRIDGE`). */
export const ORCHESTRATOR_BRUECKEN: readonly string[] = [
  "Moment, ich schaue kurz nach.",
  "Moment, ich denke kurz darüber nach.",
];

/** Überbrückung der Buch-Seite (`bookSide.ts`: `FILLER_DE`, `FILLER_EN`). */
export const BUCH_BRUECKEN: readonly string[] = [
  "Moment, ich schaue im Buch nach.",
  "One moment, I am checking the book.",
];

const BEKANNT: ReadonlySet<string> = new Set([
  ...ORCHESTRATOR_BRUECKEN,
  ...BUCH_BRUECKEN,
  ...Object.values(ANNOUNCE),
]);

function normal(text: string): string {
  return text.trim().replace(/\s+/g, " ");
}

/**
 * Ist das Stück eine Überbrückung (nicht der Inhalt der Antwort)? Nach einer Ansage (ADR-0028 A2)
 * wird sie nicht noch einmal gesprochen.
 */
export function istUeberbrueckung(text: string): boolean {
  return BEKANNT.has(normal(text));
}

/** Anführungszeichen, die beim Vergleich von Floskeln keine Rolle spielen. */
const ZITATZEICHEN = /["„“”‚‘’«»‹›]/g;

/**
 * Vergleichsform: klein, ohne Anführungszeichen, Apostroph vereinheitlicht, Leerraum gefaltet. Einfache
 * Anführungszeichen zählen nur am Wortrand als Zitatzeichen; ein Apostroph im Wort („I’m“) bleibt.
 */
function schluessel(text: string): string {
  return normal(
    text
      .toLowerCase()
      .replace(/(^|[\s(])['‘’‚`´]+/g, "$1")
      .replace(/['‘’`´]+(?=$|[\s.,;:!?…)])/g, "")
      .replace(/[’`´]/g, "'")
      .replace(ZITATZEICHEN, " "),
  );
}

/** Bekannte Überbrückungen in Vergleichsform, ohne Satzzeichen am Ende. */
const BEKANNT_SCHLUESSEL: ReadonlySet<string> = new Set(
  [...BEKANNT].map((b) => schluessel(b).replace(/[.!?…]+$/, "")),
);

/** Zitatzeichen jeder Art, auch einfache (für „Wiederholen“: ab dem ersten bleibt alles stehen). */
const JEDES_ZITATZEICHEN = /["„“”‚‘’«»‹›']/;

/**
 * Entfernt bekannte Überbrückungen aus einem längeren Text (z. B. der in Rust gemerkten Ausgabe für
 * „Wiederholen“): nur eigenständige Sätze, die genau einer Überbrückung entsprechen, und nur vor dem
 * ersten Zitatzeichen des Textes. Ab dort bleibt alles Inhalt, auch verschachtelte oder ungeschlossene
 * Zitate; lieber eine Überbrückung zu viel wiederholen als zitierten Inhalt verlieren.
 */
export function ohneUeberbrueckungen(text: string): string {
  const aus: string[] = [];
  let zitiert = false;
  for (const satz of normal(text).split(/(?<=[.!?…])\s+/)) {
    if (!satz) continue;
    zitiert ||= JEDES_ZITATZEICHEN.test(satz);
    if (!zitiert && BEKANNT.has(satz)) continue;
    aus.push(satz);
  }
  return aus.join(" ");
}

/** Adverbien, die aus „ich prüfe …“ eine Ankündigung des Wartens machen. */
const GLEICH = "(kurz|mal|eben|schnell|rasch)";
/** Verben des Nachsehens. */
const NACHSEHEN =
  "(prüfe|schaue|sehe|gucke|suche|lese|checke|kläre|überprüfe|recherchiere)";

/**
 * Was nach „ich prüfe kurz“ stehen darf: „nach“, ein Gegenstand mit Artikel oder Pronomen, ein Ort mit
 * Präposition oder „ob …“. Ein anderes Wort („kurz vor dem Schlafengehen“, „mal wieder nichts“) macht
 * den Satz zur Sachaussage.
 */
const WORAUF =
  "( nach| (das|dies|es|die|der|den|dem|deine|deinen|deinem|deiner|meine|meinen|meinem|meiner|ihre|ihren|unsere|unseren|in|im|bei|ob)( .*)?)";

/**
 * Teile eines Wartesatzes (Vergleichsform, ohne Satzzeichen). Ein Text ist nur dann eine Floskel,
 * wenn JEDER seiner Teile (getrennt an Satzzeichen und Kommas) einer davon ist; ein Sachteil
 * („Momentan ist alles gut.“, „Moment, das stimmt so nicht.“, „Ich lese jeden Abend kurz.“) macht
 * ihn zur Antwort. Die Muster verlangen die Ankündigungsform direkt nach dem Verb („ich prüfe kurz
 * …“, „ich denke kurz nach“), nie ein beliebiges Wort dazwischen.
 */
const WARTETEIL: readonly RegExp[] = [
  /^(einen |nur einen |kleinen )?(moment|augenblick)( (bitte|mal|noch))?$/,
  /^(eine |nur eine )?sekunde( bitte)?$/,
  /^(ähm|äh|hm+|um+|uh+)$/,
  // „ich prüfe kurz die Angaben“, „ich schaue mal nach“, „ich suche eben in den Mails“
  new RegExp(`^ich ${NACHSEHEN}( (das|dies|es|nach))? ${GLEICH}${WORAUF}?$`),
  // „ich schaue in den Terminen nach“, „ich sehe im Kalender nach“
  /^ich (schaue|sehe|gucke|lese) (in|im|bei) [^ ]+( [^ ]+){0,3} nach$/,
  // „ich suche in den Besprechungen“, „ich lese im Transkript“ (Werkzeug-Ansagen in Varianten)
  /^ich (suche|lese) (in|im) (den|der|dem|deinen|deiner|deinem|meinen|meiner|meinem) [^ ]+( [^ ]+)?$/,
  // „ich denke kurz nach“, „ich überlege kurz“, „ich denke kurz darüber nach“
  new RegExp(`^ich (denke|überlege) ${GLEICH}( (darüber|drüber))?( nach)?$`),
  /^ich denke( (darüber|drüber))? nach$/,
  /^(lass|lasst) (mich|uns) (kurz |mal |eben )?(nachdenken|überlegen|nachsehen|nachschauen|schauen|sehen|prüfen|nachprüfen)$/,
  /^(one|just a|wait a|give me a) (moment|second|sec|minute)( please)?$/,
  /^(hold on|hang on|wait)( a (moment|second|sec))?$/,
  /^let me (think|check|see|look)( (about it|it up|that up|into it|into that|quickly|for you))?$/,
  // „I'm checking the book“, „I'm looking into it“; nicht „I'm looking forward to it“
  /^(i'm|i am|i'll|i will) (checking|searching|check|search)( (it up|into it|into that|quickly|for you|(the|your|my) [^ ]+( [^ ]+)?))?$/,
  /^(i'm|i am|i'll|i will) (looking|look) (it up|into it|into that|(in|through) (the|your|my) [^ ]+( [^ ]+)?)$/,
  /^(i'm|i am) thinking( about it)?$/,
];

/** Ist der Satz eine Überbrückung oder eine feste Warte-Floskel (nie sprechen, nie anzeigen)? */
export function istFloskel(text: string): boolean {
  const t = schluessel(text);
  if (!t) return false;
  const saetze = t
    .split(/[.!?…]+/)
    .map((x) => x.trim())
    .filter(Boolean);
  return (
    saetze.length > 0 &&
    saetze.every((satz) => {
      if (BEKANNT_SCHLUESSEL.has(satz)) return true;
      const teile = satz
        .split(/[,;:]+/)
        .map((x) => x.trim())
        .filter(Boolean);
      return (
        teile.length > 0 && teile.every((x) => WARTETEIL.some((r) => r.test(x)))
      );
    })
  );
}
