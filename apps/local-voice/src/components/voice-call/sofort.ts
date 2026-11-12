import type { Sprache } from "./assistentPlan";

// Sofort-Sprecher (Patrick 12.11.2026, lva#485/#541): Ist das Modell des Gesprächs nicht lokal, antwortet
// zuerst ein schnelles lokales Modell. Es bekommt nur Verlauf und Frage (keinen App-Kontext, keine
// Werkzeuge) und entscheidet in einer Zeile: direkt antworten („ANTWORT:“) oder kurz weitersprechen
// („WEITER:“), während das große Modell (Denker) im Hintergrund arbeitet. Wer zuerst Inhalt liefert,
// gewinnt: Kommt der Denker vor der Entscheidung des Sprechers, wird der Sprecher verworfen.
// Reine Logik ohne Tauri-Aufrufe (Test: scripts/sofort.test.mjs); die Aufrufe stehen in der Engine.

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
  /** Unbrauchbar (leer, Format verfehlt, zu lang): nur der Denker gilt. */
  | { art: "nichts" };

const MARKE =
  /^\s*[*_"„“'`]*\s*(ANTWORT|WEITER)\s*[*_"“”'`]*\s*[:：]\s*[*_"“”'`]*\s*/i;

/** Liest die Zeile des Sprechers. Ohne Marke gilt sie nicht (lieber Stille als ein falscher Ton). */
export function liesSofort(roh: string): SofortEntscheid {
  const zeile = roh.replace(/\s+/g, " ").trim();
  const m = MARKE.exec(zeile);
  if (!m) return { art: "nichts" };
  const text = zeile
    .slice(m[0].length)
    // Eine zweite Marke (das Modell hat beide Fälle geschrieben) schneidet ab.
    .split(/\b(?:ANTWORT|WEITER)\s*[:：]/i)[0]
    .trim();
  if (text.length > SOFORT_MAX_ZEICHEN) return { art: "nichts" };
  if (m[1].toUpperCase() === "ANTWORT")
    return text ? { art: "antwort", text } : { art: "nichts" };
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
 * Inhalten erkannt (z. B. Mails); dann darf der Sprecher nie allein antworten, eine „ANTWORT“ gilt
 * dort als unbrauchbar. Liefert den neuen Zustand und den zu sprechenden Text (leer: nichts sagen).
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
    return { zustand: "denker", sagen: e.text, denkerAbbrechen: false };
  return { zustand: "denker", sagen: "", denkerAbbrechen: false };
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
