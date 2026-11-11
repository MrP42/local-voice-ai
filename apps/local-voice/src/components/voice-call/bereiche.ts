/**
 * Die Wissensbereiche des Sprachgesprächs (reine Listen, ohne Abhängigkeiten, damit Skripte und Tests
 * sie laden können). `orchestration.ts` führt sie weiter unter den alten Namen.
 */

/** Wissensbereiche des Gesprächs (Schlüssel der Einstellung `voice_call_knowledge`). */
export const KNOWLEDGE_AREAS = [
  "meetings",
  "dictations",
  "conversations",
  "calendar",
  "tts",
] as const;
export type KnowledgeArea = (typeof KNOWLEDGE_AREAS)[number];

/**
 * Bereiche nur des globalen Abrufindex (lva#325): keine Werkzeuge, nur mit dem
 * Schalter `voice_global_context_enabled`. Sie stehen in derselben Einstellung
 * `voice_call_knowledge` und bleiben beim Umschalten der Werkzeugbereiche erhalten.
 */
export const GLOBAL_AREAS = ["books", "book_notes", "ki_docs"] as const;
export type GlobalArea = (typeof GLOBAL_AREAS)[number];
/**
 * Bereich „Notizbuch“ (lva#396 N2): eigener Schalter mit Abschnittsauswahl
 * (`voice_call_notebook_sections`); ohne Abschnitt ist trotz Bereich nichts sichtbar.
 */
export const NOTEBOOK_AREA = "notebook" as const;
export type NotebookArea = typeof NOTEBOOK_AREA;
/** Alle Schlüssel in fester Reihenfolge (wie `Area::ALL` im Kern). */
export const ALL_AREAS = [
  ...KNOWLEDGE_AREAS,
  ...GLOBAL_AREAS,
  NOTEBOOK_AREA,
] as const;
