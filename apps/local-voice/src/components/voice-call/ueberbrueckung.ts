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
