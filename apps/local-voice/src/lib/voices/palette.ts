/**
 * Farbpalette der Stimmen-Registry — die Spiegelseite von `PALETTE` in
 * `src-tauri/src/managers/tts/registry.rs`. Die REIHENFOLGE ist dort Vertrag
 * (`default_color` indiziert über einen Hash in genau diese Liste); hier
 * zählt nur, dass jeder Key eine Farbe hat — deshalb eine Map statt eines
 * Arrays, denn ein verrutschter Index wäre hier still und unsichtbar.
 *
 * Die Werte sind die 500er-Stufen der Tailwind-Palette: sie tragen in Light
 * wie Dark, weil der Editor sie ohnehin nur als sehr blassen `color-mix`
 * verwendet (9 % für Textstrecken, 18 % für Chips).
 */
export const VOICE_PALETTE: Record<string, string> = {
  slate: "#64748b",
  red: "#ef4444",
  orange: "#f97316",
  amber: "#f59e0b",
  green: "#22c55e",
  teal: "#14b8a6",
  sky: "#0ea5e9",
  violet: "#8b5cf6",
  fuchsia: "#d946ef",
  rose: "#f43f5e",
};

/** Palette-Key → CSS-Farbe. Ein unbekannter Key (neuere App-Version hat die
 *  `meta.json` geschrieben) fällt auf Slate zurück, statt farblos zu bleiben. */
export const voiceColor = (key: string): string =>
  VOICE_PALETTE[key] ?? VOICE_PALETTE.slate;
