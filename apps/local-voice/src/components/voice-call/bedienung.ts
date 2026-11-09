import type { Sprache } from "./assistentPlan";

// ADR-0028 A2: lauter, leiser, schneller, langsamer führt der Sprachanruf lokal aus (S0, ohne Modell):
// Lautstärke und Tempo der Wiedergabe in festen Stufen. Reine Logik; der Player setzt die Werte um.

export type Bedienung = "lauter" | "leiser" | "schneller" | "langsamer";

/** Verstärkung der Wiedergabe je Stufe (Gain). */
export const LAUTSTAERKE: readonly number[] = [0.35, 0.5, 0.7, 1, 1.4];
/** Abspielgeschwindigkeit je Stufe (`playbackRate`; die Tonhöhe folgt ihr). */
export const TEMPO: readonly number[] = [0.8, 0.9, 1, 1.15, 1.3];
export const LAUTSTAERKE_START = 3;
export const TEMPO_START = 2;

/** Stufen der Wiedergabe eines Anrufs. */
export interface Stufen {
  lautstaerke: number;
  tempo: number;
}

export const START_STUFEN: Stufen = {
  lautstaerke: LAUTSTAERKE_START,
  tempo: TEMPO_START,
};

const ABSICHTEN: ReadonlySet<string> = new Set([
  "lauter",
  "leiser",
  "schneller",
  "langsamer",
]);

/** Die Bedienung hinter einer S0-Absicht, sonst `null`. */
export function bedienung(absicht: string): Bedienung | null {
  return ABSICHTEN.has(absicht) ? (absicht as Bedienung) : null;
}

const OK: Record<Bedienung, Record<Sprache, string>> = {
  lauter: { de: "Lauter.", en: "Louder." },
  leiser: { de: "Leiser.", en: "Quieter." },
  schneller: { de: "Schneller.", en: "Faster." },
  langsamer: { de: "Langsamer.", en: "Slower." },
};

const GRENZE: Record<Bedienung, Record<Sprache, string>> = {
  lauter: { de: "Lauter geht es nicht.", en: "I can't go any louder." },
  leiser: { de: "Leiser geht es nicht.", en: "I can't go any quieter." },
  schneller: {
    de: "Schneller geht es nicht.",
    en: "I can't go any faster.",
  },
  langsamer: {
    de: "Langsamer geht es nicht.",
    en: "I can't go any slower.",
  },
};

const NICHT_MOEGLICH = {
  lautstaerke: {
    de: "Die Lautstärke kann ich hier nicht ändern.",
    en: "I can't change the volume here.",
  },
  tempo: {
    de: "Das Tempo kann ich hier nicht ändern.",
    en: "I can't change the speed here.",
  },
} as const;

const LAUT = new Set<Bedienung>(["lauter", "leiser"]);

export interface Ergebnis {
  stufen: Stufen;
  /** Die wahre Meldung: erledigt, Grenze erreicht oder nicht möglich. */
  text: string;
  /** Wurde etwas verändert? */
  geaendert: boolean;
}

/**
 * Wendet die Bedienung an. `verfuegbar`: es gibt eine Wiedergabe, die sich stellen lässt (im Textmodus
 * ohne Anruf nicht). An der Grenze und ohne Stellgröße bleibt alles, wie es ist, und die Meldung sagt das.
 */
export function bedienen(
  art: Bedienung,
  stufen: Stufen,
  verfuegbar: boolean,
  sprache: Sprache,
): Ergebnis {
  const laut = LAUT.has(art);
  if (!verfuegbar) {
    const m = laut ? NICHT_MOEGLICH.lautstaerke : NICHT_MOEGLICH.tempo;
    return { stufen, text: m[sprache], geaendert: false };
  }
  const schritt = art === "lauter" || art === "schneller" ? 1 : -1;
  const alt = laut ? stufen.lautstaerke : stufen.tempo;
  const max = (laut ? LAUTSTAERKE : TEMPO).length - 1;
  const neu = alt + schritt;
  if (neu < 0 || neu > max)
    return { stufen, text: GRENZE[art][sprache], geaendert: false };
  return {
    stufen: laut ? { ...stufen, lautstaerke: neu } : { ...stufen, tempo: neu },
    text: OK[art][sprache],
    geaendert: true,
  };
}
