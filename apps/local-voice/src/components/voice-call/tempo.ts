import { DEFAULT_TURN_CONFIG, type TurnConfig } from "./turnTaking";

export type Tempo = "patient" | "balanced" | "fast";
export type TempoChoice = Tempo | "custom";

/** Antworttempo = Paar aus Pause bis Satzende und früher Vorarbeit. */
export const TEMPO_PRESETS: Record<
  Tempo,
  Pick<TurnConfig, "endMs" | "earlyEndMs">
> = {
  patient: { endMs: 1000, earlyEndMs: 500 },
  balanced: {
    endMs: DEFAULT_TURN_CONFIG.endMs,
    earlyEndMs: DEFAULT_TURN_CONFIG.earlyEndMs,
  },
  fast: { endMs: 550, earlyEndMs: 280 },
};

export const TEMPOS: Tempo[] = ["patient", "balanced", "fast"];

export function tempoOf(
  turn: Pick<TurnConfig, "endMs" | "earlyEndMs">,
): TempoChoice {
  const hit = TEMPOS.find(
    (k) =>
      TEMPO_PRESETS[k].endMs === turn.endMs &&
      TEMPO_PRESETS[k].earlyEndMs === turn.earlyEndMs,
  );
  return hit ?? "custom";
}
