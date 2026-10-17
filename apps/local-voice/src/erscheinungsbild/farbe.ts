/**
 * @wai/ui erscheinungsbild v1 — farbe.ts
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Port aus WAI Presenter, erscheinungsbild/farbe.ts).
 * Selbsttragend: nur reine Funktionen, keine Importe. Zum Kopieren in Apps ohne @wai/ui geeignet.
 *
 * Farbrechnung fuer Akzente und Kontrastpruefung (WCAG 2.x) auf Hex-Werten `#RRGGBB` sowie die Erzeugung
 * einer Fluent-`BrandVariants`-Rampe (Stufen 10..160) aus einer einzelnen Markenfarbe.
 */

export type Rgb = readonly [number, number, number];

/** Mindestkontrast nach WCAG AA: Text 4,5:1, grosse Schrift und Bedienelemente 3:1. */
export const KONTRAST_TEXT = 4.5;
export const KONTRAST_ELEMENT = 3;

export const WEISS = "#FFFFFF";
export const SCHWARZ = "#000000";
/** Fast-Schwarz fuer Schrift auf hellen Markenflaechen (WAI-Ink). */
export const TINTE = "#111418";

const HEX = /^#([0-9a-f]{6})$/i;

export function istHex(wert: unknown): wert is string {
  return typeof wert === "string" && HEX.test(wert);
}

export function hexZuRgb(hex: string): Rgb {
  const m = HEX.exec(hex);
  if (!m) throw new Error(`Keine Farbe im Format #RRGGBB: ${hex}`);
  const n = Number.parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

const zweistellig = (n: number) =>
  Math.max(0, Math.min(255, Math.round(n)))
    .toString(16)
    .padStart(2, "0");

export function rgbZuHex([r, g, b]: Rgb): string {
  return `#${zweistellig(r)}${zweistellig(g)}${zweistellig(b)}`.toUpperCase();
}

const linear = (k: number) => {
  const s = k / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
};

/** Relative Leuchtdichte nach WCAG. */
export function leuchtdichte(hex: string): number {
  const [r, g, b] = hexZuRgb(hex);
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

/** Kontrastverhaeltnis zweier Farben (1 bis 21). */
export function kontrast(a: string, b: string): number {
  const la = leuchtdichte(a);
  const lb = leuchtdichte(b);
  const [hell, dunkel] = la >= lb ? [la, lb] : [lb, la];
  return (hell + 0.05) / (dunkel + 0.05);
}

/** Mischt `a` mit `b`; `anteil` 0 ergibt `a`, 1 ergibt `b`. */
export function mische(a: string, b: string, anteil: number): string {
  const t = Math.max(0, Math.min(1, anteil));
  const [r1, g1, b1] = hexZuRgb(a);
  const [r2, g2, b2] = hexZuRgb(b);
  return rgbZuHex([r1 + (r2 - r1) * t, g1 + (g2 - g1) * t, b1 + (b2 - b1) * t]);
}

/** Schlechtester Kontrast einer Farbe gegen mehrere Flaechen. */
export function minKontrast(farbe: string, flaechen: readonly string[]): number {
  return Math.min(...flaechen.map((f) => kontrast(farbe, f)));
}

/**
 * Verschiebt `farbe` in kleinen Schritten Richtung `ziel` (Weiss oder Schwarz), bis sie gegen alle `flaechen`
 * mindestens `mindest` erreicht. Erreicht sie das nie, kommt das Ziel selbst zurueck.
 */
export function verschiebeBisKontrast(
  farbe: string,
  ziel: string,
  flaechen: readonly string[],
  mindest: number,
): string {
  if (minKontrast(farbe, flaechen) >= mindest) return farbe.toUpperCase();
  for (let schritt = 1; schritt <= 50; schritt += 1) {
    const kandidat = mische(farbe, ziel, schritt / 50);
    if (minKontrast(kandidat, flaechen) >= mindest) return kandidat;
  }
  return ziel;
}

/** Besser lesbare Schriftfarbe auf einer Flaeche: Weiss oder das dunkle `dunkel`. */
export function textAuf(flaeche: string, dunkel = TINTE): string {
  return kontrast(WEISS, flaeche) >= kontrast(dunkel, flaeche) ? WEISS : dunkel;
}

/** Stufen einer Fluent-`BrandVariants`-Rampe. */
export const RAMPEN_STUFEN = [
  10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160,
] as const;
export type RampenStufe = (typeof RAMPEN_STUFEN)[number];
/** Strukturgleich zu Fluents `BrandVariants` (ohne Import, damit die Datei selbsttragend bleibt). */
export type Rampe = Record<RampenStufe, string>;

/**
 * Fluent-Rampe aus einer Farbe: Stufe `anker` ist die Farbe selbst, darunter wird gleichmaessig zu fast
 * Schwarz, darueber zu fast Weiss gemischt. Fluent nutzt im hellen Theme Stufe 80, im dunklen Stufe 100
 * fuer Markentext — deshalb ist das der passende Anker je Schema.
 */
export function brandRampe(hex: string, anker: RampenStufe = 80): Rampe {
  const basis = hex.toUpperCase();
  const r = {} as Rampe;
  for (const s of RAMPEN_STUFEN) {
    if (s === anker) r[s] = basis;
    else if (s < anker) r[s] = mische(basis, SCHWARZ, ((anker - s) / anker) * 0.9);
    else r[s] = mische(basis, WEISS, ((s - anker) / (170 - anker)) * 0.88);
  }
  return r;
}
