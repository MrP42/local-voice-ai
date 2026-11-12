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

/**
 * Entfernt bekannte Überbrückungen aus einem längeren Text (z. B. der in Rust gemerkten Ausgabe für
 * „Wiederholen“ oder der Rückgabe des Tors): nur der Inhalt bleibt.
 */
export function ohneUeberbrueckungen(text: string): string {
  let rest = normal(text);
  for (const b of BEKANNT) rest = rest.split(b).join(" ");
  return normal(rest);
}

/** Feste Floskeln des Wartens, auch frei formuliert („Ich denke kurz nach.“, „Let me check.“). */
const FLOSKEL =
  /^(moment|einen moment|augenblick|sekunde|ich (denke|überlege|schaue|sehe|prüfe|suche|gucke)\b[^.!?]*\b(nach|kurz|darüber)\b|lass mich|one moment|just a (moment|second|sec)|hold on|let me (think|check|see|look)|i('m| am) (thinking|checking|looking))/i;

/** Ist der Satz eine Überbrückung oder eine feste Warte-Floskel (nie sprechen, nie anzeigen)? */
export function istFloskel(text: string): boolean {
  const t = normal(text);
  return istUeberbrueckung(t) || FLOSKEL.test(t);
}
