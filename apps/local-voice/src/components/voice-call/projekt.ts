/**
 * Gespräch im Projekt (lva#365 P4c-2): Hinweise aus dem Projektweg und die Bindung des Gesprächs.
 *
 * Die Hinweise tragen nur Codes, Zähler und Anteile des Budgets, nie Inhalte. Die Bindung (`id`) stammt
 * aus dem gespeicherten Gespräch (`scope.project_id`), nicht aus einer Eingabe der Oberfläche; der
 * Kern in Rust liest sie ohnehin selbst aus dem Speicher.
 */

/** Anteile des Budgets beim Stopp „zu viel Kontext“ (Token). */
export interface ProjektBudget {
  fenster: number;
  benoetigt: number;
  system: number;
  kontext: number;
  historie: number;
  auszuege: number;
  zwischen: number;
  frage: number;
  puffer: number;
  ausgabe: number;
}

export interface ProjektHinweis {
  /** `projekt.budget`, `projekt.stand_veraltet`, `projekt.zurueckgesetzt`, … */
  code: string;
  /** Der Verlauf wurde verworfen: nur die neue Frage ging an das Modell. */
  zurueckgesetzt?: boolean;
  /** Anzahl ersetzter oder verworfener Antworten und Blöcke. */
  entzogen?: number;
  budget?: ProjektBudget | null;
}

/** Das Projekt des offenen Gesprächs und der Stand der Auswahl, den die Person gesehen hat. */
export interface ProjektBindung {
  id: string;
  /** `stand` der Übersicht; weicht er beim Senden ab, antwortet das Tor mit `projekt.stand_veraltet`. */
  stand: string | null;
}

/** Codes, die nur informieren (die Antwort kam trotzdem); alle anderen stoppen den Versand. */
export const INFO_CODES: readonly string[] = [
  "projekt.zurueckgesetzt",
  "projekt.entzogen",
];

export const istInfo = (h: ProjektHinweis): boolean =>
  INFO_CODES.includes(h.code);

const CODE = /^(projekt\.[a-z0-9_]+):/;

/** Der Code einer abgelehnten Anfrage (`<code>: <Text>`), sonst `null`. */
export function codeAusFehler(e: unknown): string | null {
  const text = e instanceof Error ? e.message : String(e ?? "");
  return CODE.exec(text.trim())?.[1] ?? null;
}

/** Hinweis aus der Ablehnung einer Anfrage; ohne Projektcode `null`. */
export function hinweisAusFehler(e: unknown): ProjektHinweis | null {
  const code = codeAusFehler(e);
  return code ? { code } : null;
}

/** Hinweis aus dem Ereignis `projekt` des Kerns. */
export function hinweisAusEreignis(e: {
  code?: string;
  zurueckgesetzt?: boolean;
  entzogen?: number;
  budget?: ProjektBudget | null;
}): ProjektHinweis | null {
  if (!e.code || !e.code.startsWith("projekt.")) return null;
  return {
    code: e.code,
    zurueckgesetzt: e.zurueckgesetzt === true,
    entzogen: typeof e.entzogen === "number" ? e.entzogen : 0,
    budget: e.budget ?? null,
  };
}

/**
 * Führt zwei Hinweise zum selben Vorgang zusammen: Das Ereignis (mit Budgetanteilen) und die
 * Ablehnung (nur Code) treffen in beliebiger Reihenfolge ein; der reichere gewinnt.
 */
export function zusammen(
  alt: ProjektHinweis | null,
  neu: ProjektHinweis,
): ProjektHinweis {
  if (alt && alt.code === neu.code)
    return { ...alt, ...neu, budget: neu.budget ?? alt.budget ?? null };
  return neu;
}
