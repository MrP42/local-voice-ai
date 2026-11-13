import type { Sprache } from "./assistentPlan";
import { istFloskel } from "./ueberbrueckung";

// Sofort-Sprecher (Patrick 12.11.2026, lva#485/#541): Ist das Modell des Gesprächs nicht lokal, antwortet
// zuerst ein schnelles lokales Modell. Es bekommt nur Verlauf und Frage (keinen App-Kontext, keine
// Werkzeuge) und entscheidet in einer Zeile: direkt antworten („ANTWORT:“) oder kurz weitersprechen
// („WEITER:“), während das große Modell (Denker) im Hintergrund arbeitet. Wer zuerst Inhalt liefert,
// gewinnt: Kommt der Denker vor der Entscheidung des Sprechers, wird der Sprecher verworfen.
// Reine Logik ohne Tauri-Aufrufe (Test: scripts/sofort.test.mjs); die Aufrufe stehen in der Engine.
//
// Offener Auftrag (ADR-0028 E4.2c 7(e)): Hat der Sprecher „WEITER“ gesagt und liefert der Denker noch nichts,
// beendet eine weitere Äußerung nur den Vordergrund; der Denker-Lauf bleibt ein offener Auftrag. Die neue
// Äußerung geht dann zuerst allein an den Sprecher, mit dem Block „offener Suchauftrag“ (`auftragsBlock`):
// „ANTWORT“ (Auftrag bleibt), „SUCHE“ (Auftrag ersetzt) oder „ABBRUCH“ (Auftrag verworfen). Der Block geht
// nur an das lokale Modell, nie an einen Denker.

/** Höchstwartezeit auf die Entscheidung des Sprechers; danach gilt nur der Denker. */
export const SOFORT_MAX_MS = 4000;

/** So viele Beiträge des Verlaufs sieht der Sprecher (je Beitrag Nutzer oder KI). */
export const SOFORT_VERLAUF = 8;

/** Höchstlänge der Sprecher-Zeile, die gesprochen wird (Zeichen); Längeres ist kein „kurz“. */
export const SOFORT_MAX_ZEICHEN = 280;

const PROMPT: Record<Sprache, string> = {
  de: [
    "Du bist die schnelle Stimme eines Sprachassistenten im Gespräch. Ein gründlicheres Modell arbeitet",
    "gleichzeitig an der Antwort und kann auf die eigenen Inhalte zugreifen (Mails, Termine, Besprechungen,",
    "Dokumente, Bücher); du kannst das nicht. Schreibe genau eine Zeile ohne Markdown.",
    "- Ist die Äußerung eine Begrüßung, Smalltalk, eine Hörprobe, ein Dank, eine Bestätigung oder eine Frage,",
    "  die du sicher und vollständig ohne Nachschlagen beantworten kannst: schreibe „ANTWORT:“ und die",
    "  Antwort in höchstens zwei kurzen, gesprochenen Sätzen.",
    "- Sonst, und immer bei Fragen zu den eigenen Inhalten, aktuellen Daten oder Fakten, bei denen du",
    "  unsicher bist: schreibe „WEITER:“ und höchstens einen kurzen, natürlichen Satz, der auf den Inhalt",
    "  eingeht, ohne ihn zu beantworten und ohne etwas zu erfinden. Keine Floskeln wie „Moment“, „Ich denke",
    "  nach“ oder „Ich schaue nach“, keine Quellenangaben. Passt kein Satz, schreibe nur „WEITER:“.",
    "Sprich die Person mit du an und nenne keinen Namen, den du nicht aus dem Gespräch kennst.",
  ].join("\n"),
  en: [
    "You are the fast voice of a voice assistant in a conversation. A more thorough model is working on the",
    "answer at the same time and can access the user's own content (mail, calendar, meetings, documents,",
    "books); you cannot. Write exactly one line without markdown.",
    "- If the utterance is a greeting, small talk, a sound check, thanks, a confirmation or a question you can",
    "  answer reliably and completely without looking anything up: write “ANTWORT:” and the answer in at most",
    "  two short spoken sentences.",
    "- Otherwise, and always for questions about the user's own content, current data or facts you are",
    "  unsure of: write “WEITER:” and at most one short, natural sentence that engages with the content",
    "  without answering it and without inventing anything. No filler such as “one moment”, “let me think”",
    "  or “let me check”, no sources. If no sentence fits, write only “WEITER:”.",
    "Do not use a name you do not know from the conversation.",
  ].join("\n"),
};

/** Systemanweisung des Sprechers in der Sprache der App. */
export function sofortPrompt(sprache: Sprache): string {
  return PROMPT[sprache];
}

export type SofortEntscheid =
  /** Vollständige Antwort: der Denker wird abgebrochen. */
  | { art: "antwort"; text: string }
  /** Kurz weitersprechen (`text` darf leer sein); der Denker liefert danach. */
  | { art: "weiter"; text: string }
  /** Nur mit offenem Auftrag: Ergänzung, Korrektur oder neue Inhaltsfrage (`text` darf leer sein). */
  | { art: "suche"; text: string }
  /** Nur mit offenem Auftrag: die Suche ist nicht mehr gewollt (`text` darf leer sein). */
  | { art: "abbruch"; text: string }
  /** Unbrauchbar (leer, Format verfehlt, zu lang): nur der Denker gilt. */
  | { art: "nichts" };

const MARKE =
  /^\s*[*_"„“'`]*\s*(ANTWORT|WEITER|SUCHE|ABBRUCH)\s*[*_"“”'`]*\s*[:：]\s*[*_"“”'`]*\s*/i;

/**
 * Liest die Zeile des Sprechers. Ohne Marke gilt sie nicht (lieber Stille als ein falscher Ton).
 * `auftragOffen`: „SUCHE“ und „ABBRUCH“ gelten nur, wenn ein Auftrag offen ist (sonst `nichts`); dann
 * gilt „WEITER“ nicht (der Sprecher kennt es in diesem Fall nicht).
 */
export function liesSofort(roh: string, auftragOffen = false): SofortEntscheid {
  const zeile = roh.replace(/\s+/g, " ").trim();
  const m = MARKE.exec(zeile);
  if (!m) return { art: "nichts" };
  const marke = m[1].toUpperCase();
  const auftragsMarke = marke === "SUCHE" || marke === "ABBRUCH";
  if (auftragsMarke && !auftragOffen) return { art: "nichts" };
  if (marke === "WEITER" && auftragOffen) return { art: "nichts" };
  const text = zeile
    .slice(m[0].length)
    // Eine zweite Marke (das Modell hat beide Fälle geschrieben) schneidet ab: nur in Großbuchstaben
    // und am Satzanfang, damit „Es heißt Weiter: …“ im Inhalt ganz bleibt.
    .split(/(?:^|(?<=[.!?…])\s+)(?:ANTWORT|WEITER|SUCHE|ABBRUCH)\s*[:：]/)[0]
    .trim();
  if (text.length > SOFORT_MAX_ZEICHEN) return { art: "nichts" };
  // Feste Floskeln („Ich denke kurz nach.“) sind nie eine Reaktion: lieber Stille.
  if (text && istFloskel(text)) return { art: "nichts" };
  if (marke === "ANTWORT")
    return text ? { art: "antwort", text } : { art: "nichts" };
  if (marke === "SUCHE") return { art: "suche", text };
  if (marke === "ABBRUCH") return { art: "abbruch", text };
  return { art: "weiter", text };
}

/**
 * Wer gewinnt? `zustand` folgt dem ersten entscheidenden Ereignis:
 * - `wartet`: noch nichts; Denker-Inhalt beendet das Warten zugunsten des Denkers.
 * - `denker`: der Denker spricht (allein oder nach dem Sprecher).
 * - `sprecher`: der Sprecher hat vollständig geantwortet, der Denker wird verworfen.
 */
export type SofortZustand = "wartet" | "denker" | "sprecher";

/**
 * Entscheidung des Sprechers anwenden. `datenAbsicht`: die Einstufung hat eine Absicht zu eigenen
 * Inhalten erkannt (z. B. Mails); dann sagt der Sprecher gar nichts (weder „ANTWORT“ noch „WEITER“),
 * weil er diese Inhalte nicht kennt und jede Sachaussage ungeprüft wäre. Liefert den neuen Zustand
 * und den zu sprechenden Text (leer: nichts sagen).
 */
export function wendeAn(
  zustand: SofortZustand,
  e: SofortEntscheid,
  datenAbsicht: boolean,
): { zustand: SofortZustand; sagen: string; denkerAbbrechen: boolean } {
  if (zustand !== "wartet")
    return { zustand, sagen: "", denkerAbbrechen: false };
  if (e.art === "antwort" && !datenAbsicht)
    return { zustand: "sprecher", sagen: e.text, denkerAbbrechen: true };
  if (e.art === "weiter")
    return {
      zustand: "denker",
      sagen: datenAbsicht ? "" : e.text,
      denkerAbbrechen: false,
    };
  return { zustand: "denker", sagen: "", denkerAbbrechen: false };
}

/** Höchstlänge der ursprünglichen Frage im Block „offener Suchauftrag“ (Zeichen). */
export const AUFTRAG_FRAGE_MAX = 400;

const BLOCK: Record<Sprache, (frage: string, ergebnis: string) => string> = {
  de: (frage, ergebnis) =>
    [
      `Offener Suchauftrag: Das gründlichere Modell sucht noch zu dieser Frage: „${frage}“. Ergebnis liegt vor: ${ergebnis}.`,
      "Die neue Äußerung des Gesprächspartners kann damit zusammenhängen. Schreibe genau eine Zeile ohne Markdown",
      "und beginne mit genau einer Marke („WEITER:“ gibt es hier nicht):",
      "- „ANTWORT:“ und eine kurze Antwort in höchstens zwei gesprochenen Sätzen, wenn die Äußerung etwas",
      "  Eigenständiges ist, das du sicher ohne Nachschlagen beantworten kannst (Begrüßung, Dank, Smalltalk,",
      "  Frage nach dem Stand). Die Suche läuft weiter.",
      "- „SUCHE:“ (optional ein kurzer, natürlicher Satz dahinter), wenn die Äußerung die Frage ergänzt oder",
      "  korrigiert oder eine neue Frage zu den eigenen Inhalten, aktuellen Daten oder Fakten stellt, bei denen",
      "  du unsicher bist. Die alte Suche wird dann verworfen und neu gesucht.",
      "- „ABBRUCH:“ (optional eine kurze Bestätigung dahinter), wenn der Gesprächspartner die Suche nicht mehr will.",
      "Du kennst das Ergebnis nicht: sage nichts darüber und erfinde nichts. Keine Floskeln wie „Moment“ oder",
      "„Ich schaue nach“.",
    ].join("\n"),
  en: (frage, ergebnis) =>
    [
      `Open search job: The more thorough model is still searching for this question: “${frage}”. Result available: ${ergebnis}.`,
      "The other person's new utterance may be related. Write exactly one line without markdown and start with",
      "exactly one marker (“WEITER:” does not exist here):",
      "- “ANTWORT:” and a short answer in at most two spoken sentences if the utterance is something",
      "  self-contained that you can answer reliably without looking anything up (greeting, thanks, small talk,",
      "  asking for the status). The search keeps running.",
      "- “SUCHE:” (optionally one short, natural sentence after it) if the utterance adds to or corrects the",
      "  question or asks a new question about the user's own content, current data or facts you are unsure of.",
      "  The old search is then dropped and started again.",
      "- “ABBRUCH:” (optionally a short confirmation after it) if the other person no longer wants the search.",
      "You do not know the result: say nothing about it and invent nothing. No filler such as “one moment” or",
      "“let me check”.",
    ].join("\n"),
};

/**
 * Systemblock „offener Suchauftrag“ für den Sprecher: ursprüngliche Frage und ob ein Ergebnis da ist
 * (ja/nein, nie dessen Inhalt). Er geht nur an das lokale Modell.
 */
export function auftragsBlock(
  frage: string,
  ergebnisDa: boolean,
  sprache: Sprache,
): string {
  const f = frage.replace(/\s+/g, " ").trim();
  const kurz =
    f.length > AUFTRAG_FRAGE_MAX ? `${f.slice(0, AUFTRAG_FRAGE_MAX)}…` : f;
  const ja = sprache === "de" ? "ja" : "yes";
  const nein = sprache === "de" ? "nein" : "no";
  return BLOCK[sprache](kurz, ergebnisDa ? ja : nein);
}

/** Entscheidung des Sprechers bei offenem Auftrag, bereinigt um die Datenabsicht. */
export type AuftragEntscheid =
  /** Kurze Antwort auf die neue Äußerung; der Auftrag bleibt. */
  | { art: "antwort"; sagen: string }
  /** Der Auftrag wird verworfen, für die neue Äußerung startet ein neuer Denker (`sagen` darf leer sein). */
  | { art: "suche"; sagen: string }
  /** Der Auftrag wird verworfen, nichts Neues startet (`sagen`: Bestätigung, darf leer sein). */
  | { art: "abbruch"; sagen: string };

/**
 * Entscheidung des Sprechers bei offenem Auftrag anwenden (keine Heuristik, die Marke entscheidet).
 * Unbrauchbare Zeile, Floskel, Fehler und Zeitüberschreitung (`nichts`) gelten wie „SUCHE“. Bei
 * Datenabsicht der Einstufung gilt jede Marke wie „SUCHE“ ohne Text (der Sprecher sagt bei Datenabsicht
 * nichts, wie bei `wendeAn`): die neue Äußerung ist eine Datenfrage und gehört dem Denker.
 */
export function wendeAuftragAn(
  e: SofortEntscheid,
  datenAbsicht: boolean,
): AuftragEntscheid {
  if (datenAbsicht) return { art: "suche", sagen: "" };
  if (e.art === "antwort") return { art: "antwort", sagen: e.text };
  if (e.art === "abbruch") return { art: "abbruch", sagen: e.text };
  if (e.art === "suche") return { art: "suche", sagen: e.text };
  return { art: "suche", sagen: "" };
}

/** Modell, wie die Orchestrierung es beschreibt (nur die hier nötigen Felder). */
export interface SofortKandidat {
  id: string;
  local: boolean;
}

/**
 * Lokales Modell für den Sofort-Sprecher: nur, wenn das Modell des Gesprächs nicht lokal ist. Gewählt
 * wird das Modell der Funktion `voice_sofort`, wenn es lokal ist, sonst das erste lokale Modell.
 */
export function sofortModell<T extends SofortKandidat>(
  active: T | null,
  all: T[],
  gewaehlt: string | null,
): T | null {
  if (!active || active.local) return null;
  const lokal = all.filter((m) => m.local && m.id !== active.id);
  return lokal.find((m) => m.id === gewaehlt) ?? lokal[0] ?? null;
}
