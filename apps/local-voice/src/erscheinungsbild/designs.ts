/**
 * @wai/ui erscheinungsbild v1.1 — designs.ts
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: WAI Presenter, erscheinungsbild/designs.ts).
 * Selbsttragend: importiert nur ./farbe. Zum Kopieren in Apps ohne @wai/ui geeignet.
 *
 * Register der App-Designs und Akzente. Jede App hat den Windows-11-/Fluent-2-Aufbau; ein Design legt nur
 * Markenfarbe, Flaechen, Radius und Schrift fest. Standard ist `microsoft` (Fluent webLight/webDark
 * unveraendert), `wai` ist optional. Apps reichen weitere Designs als Liste an `useErscheinungsbild` weiter.
 */

export type Schema = "hell" | "dunkel";

/** Flaechen eines Schemas; jede Akzentfarbe wird gegen alle vier auf 4,5:1 gebracht. */
export interface DesignFlaechen {
  /** Seitengrund (Fluent colorNeutralBackground2). */
  hintergrund: string;
  /** Abgesetzte Flaeche, z. B. Leisten (colorNeutralBackground3). */
  unterflaeche: string;
  /** Karten und Bedienelemente (colorNeutralBackground1). */
  karte: string;
  /** Navigationsbereich (colorNeutralBackground4). */
  navigation: string;
}

export type DesignRadius = "eckig" | "standard" | "rund";

export interface AppDesign {
  /** Stabile Kennung (Kleinbuchstaben, Ziffern, Bindestrich); wird gespeichert. */
  id: string;
  name: string;
  /** Eine Zeile fuer die Auswahl: wofuer das Design gedacht ist. */
  beschreibung: string;
  /** Markenfarbe je Schema (`#RRGGBB`). Wird bei Bedarf bis 4,5:1 gegen die Flaechen verschoben. */
  marke: Record<Schema, string>;
  /** Eigene Flaechen; ohne Angabe gelten die Fluent-Neutraltoene. */
  flaechen?: Record<Schema, DesignFlaechen>;
  radius?: DesignRadius;
  /** CSS-Schriftfamilie fuer Text; ohne Angabe Segoe UI (Fluent). */
  schrift?: string;
  /**
   * Dunkles Schema: helle Markenfarbe als Flaeche mit dunkler Schrift (WAI-Signalgelb) statt der
   * Fluent-Abdunklung mit weisser Schrift.
   */
  markeAlsFlaecheDunkel?: boolean;
}

export const MICROSOFT: AppDesign = {
  id: "microsoft",
  // Anzeigename ohne fremde Produktnamen (Leitlinie windows-fluent §1: keine suggerierte Zugehoerigkeit).
  name: "Fluent",
  beschreibung: "Standard: Fluent 2 mit neutralem Grau und Blau, passend zur Systemoberfläche.",
  marke: { hell: "#0F6CBD", dunkel: "#479EF5" },
  radius: "standard",
};

export const WAI: AppDesign = {
  id: "wai",
  name: "WAI",
  beschreibung: "Wolff Applied AI: Navy im hellen, Signalgelb im dunklen Modus.",
  marke: { hell: "#1F3A5F", dunkel: "#FFDD00" },
  flaechen: {
    hell: {
      hintergrund: "#F8F9FB",
      unterflaeche: "#EEF2F7",
      karte: "#FFFFFF",
      navigation: "#E5E9EF",
    },
    dunkel: {
      hintergrund: "#111215",
      unterflaeche: "#0B0B0C",
      karte: "#15161A",
      navigation: "#08080A",
    },
  },
  radius: "standard",
  schrift: '"Space Grotesk", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
  markeAlsFlaecheDunkel: true,
};

/** Eingebaute Designs; das erste ist der Standard. */
export const EINGEBAUTE_DESIGNS: readonly AppDesign[] = [MICROSOFT, WAI];
export const STANDARD_DESIGN_ID = MICROSOFT.id;

/** Eingebaute plus App-Designs; ein App-Design mit gleicher Kennung ersetzt das eingebaute. */
export function designListe(zusatz: readonly AppDesign[] = []): AppDesign[] {
  const liste = EINGEBAUTE_DESIGNS.filter((d) => !zusatz.some((z) => z.id === d.id));
  return [...liste, ...zusatz];
}

// ------------------------------------------------------------------------------------------------ Akzente

export const AKZENT_STANDARD = "standard";

export interface AkzentDef {
  id: string;
  name: string;
  /** Ausgangsfarbe (Windows-11-Akzentfarben); `null`: Markenfarbe des Designs. */
  basis: string | null;
}

export const AKZENTE: readonly AkzentDef[] = [
  { id: AKZENT_STANDARD, name: "Wie das Design", basis: null },
  { id: "blau", name: "Blau", basis: "#0078D4" },
  { id: "petrol", name: "Petrol", basis: "#038387" },
  { id: "gruen", name: "Grün", basis: "#107C10" },
  { id: "violett", name: "Violett", basis: "#8764B8" },
  { id: "orange", name: "Orange", basis: "#CA5010" },
  { id: "beere", name: "Beere", basis: "#C239B3" },
  { id: "rot", name: "Rot", basis: "#D13438" },
  { id: "graphit", name: "Graphit", basis: "#69797E" },
];

export function istAkzentId(wert: unknown): wert is string {
  return typeof wert === "string" && AKZENTE.some((a) => a.id === wert);
}

/** Basisfarbe eines Akzents; `undefined` fuer „Wie das Design“ oder unbekannte Kennungen. */
export function akzentBasis(akzentId: string | undefined): string | undefined {
  return AKZENTE.find((a) => a.id === akzentId)?.basis ?? undefined;
}
