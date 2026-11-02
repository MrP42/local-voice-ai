/*
 * Gemeinsame Typen der Projekt- und Recherche-Bausteine (ADR-0026). Die Werte folgen den Verträgen
 * `project`, `project-entry` und `research` (contracts/core); Feldnamen sind camelCase (`origin_app` → `originApp`).
 */

/** Datenklasse nach Vertrag (`DataClass`). */
export type Datenklasse = "PUBLIC" | "INTERNAL" | "PERSONAL" | "SPECIAL" | "RESTRICTED";

/** Art eines Projekteintrags (`project-entry.kind`). */
export type EintragArt =
  | "meeting"
  | "transcript"
  | "conversation"
  | "book"
  | "book_document"
  | "note"
  | "research"
  | "document"
  | "plan"
  | "talk";

export type RechercheModus = "quick" | "thorough" | "deep";
export type QuellenUmfang = "web" | "project" | "both";
export type QuellenArt = "official" | "scholarly" | "press" | "forum" | "unknown";

/** Klassen, die Hinweisfarbe und Warnton bekommen. */
export const SENSIBLE_KLASSEN: readonly Datenklasse[] = ["PERSONAL", "SPECIAL", "RESTRICTED"];

export const istSensibel = (klasse: Datenklasse) => SENSIBLE_KLASSEN.includes(klasse);
