/**
 * @wai/ui erscheinungsbild v1.1 — theme.ts
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild.
 * Selbsttragend: importiert nur @fluentui/react-components und ./designs, ./farbe.
 *
 * Aus Design, Schema und Optionen wird ein Fluent-2-`Theme`. Regeln:
 * - Microsoft ohne Akzent liefert exakt `webLightTheme` / `webDarkTheme` (Original-Fluent).
 * - Kontrast „hoch“ liefert `teamsHighContrastTheme`; Akzent und Design ruhen dann.
 * - Sonst: Markenfarbe (oder Akzent) wird bis 4,5:1 gegen alle vier Flaechen verschoben, daraus eine
 *   `BrandVariants`-Rampe erzeugt (createLightTheme/createDarkTheme) und Flaechen, Radius, Schrift gesetzt.
 */
import {
  type BrandVariants,
  createDarkTheme,
  createLightTheme,
  type Theme,
  teamsHighContrastTheme,
  webDarkTheme,
  webLightTheme,
} from "@fluentui/react-components";
import {
  type AppDesign,
  akzentBasis,
  type DesignFlaechen,
  MICROSOFT,
  type Schema,
} from "./designs";
import {
  brandRampe,
  istHex,
  KONTRAST_TEXT,
  mische,
  SCHWARZ,
  textAuf,
  verschiebeBisKontrast,
  WEISS,
} from "./farbe";

export type Kontrast = "standard" | "hoch";

export interface ThemeOptionen {
  /** Akzent: Kennung aus `AKZENTE` oder eine Farbe `#RRGGBB`; leer oder „standard“: Markenfarbe. */
  akzent?: string;
  kontrast?: Kontrast;
}

/** Die vier Flaechen eines Fluent-Themes (Zuordnung siehe `DesignFlaechen`). */
export function flaechenVon(theme: Theme): DesignFlaechen {
  return {
    karte: theme.colorNeutralBackground1,
    hintergrund: theme.colorNeutralBackground2,
    unterflaeche: theme.colorNeutralBackground3,
    navigation: theme.colorNeutralBackground4,
  };
}

/** Flaechen, gegen die Akzente in diesem Design und Schema lesbar sein muessen. */
export function wirksameFlaechen(design: AppDesign, schema: Schema): DesignFlaechen {
  return (
    design.flaechen?.[schema] ?? flaechenVon(schema === "dunkel" ? webDarkTheme : webLightTheme)
  );
}

const liste = (f: DesignFlaechen) => [f.hintergrund, f.unterflaeche, f.karte, f.navigation];

/**
 * Markenfarbe, wie sie im Theme erscheint: Akzent oder Design-Marke, verschoben bis Text auf allen Flaechen
 * mindestens 4,5:1 erreicht (kleiner Puffer gegen Rundung).
 */
export function wirksameMarke(design: AppDesign, schema: Schema, akzent?: string): string {
  const roh = (istHex(akzent) ? akzent : akzentBasis(akzent)) ?? design.marke[schema];
  const ziel = schema === "dunkel" ? WEISS : SCHWARZ;
  return verschiebeBisKontrast(
    roh,
    ziel,
    liste(wirksameFlaechen(design, schema)),
    KONTRAST_TEXT + 0.1,
  );
}

function flaechenTokens(f: DesignFlaechen, schema: Schema): Partial<Theme> {
  const ziel = schema === "dunkel" ? WEISS : SCHWARZ;
  const stufen = (basis: string) => ({
    hover: mische(basis, ziel, 0.04),
    pressed: mische(basis, ziel, 0.09),
    selected: mische(basis, ziel, 0.06),
  });
  const k = stufen(f.karte);
  const h = stufen(f.hintergrund);
  const u = stufen(f.unterflaeche);
  const n = stufen(f.navigation);
  return {
    colorNeutralBackground1: f.karte,
    colorNeutralBackground1Hover: k.hover,
    colorNeutralBackground1Pressed: k.pressed,
    colorNeutralBackground1Selected: k.selected,
    colorNeutralBackground2: f.hintergrund,
    colorNeutralBackground2Hover: h.hover,
    colorNeutralBackground2Pressed: h.pressed,
    colorNeutralBackground2Selected: h.selected,
    colorNeutralBackground3: f.unterflaeche,
    colorNeutralBackground3Hover: u.hover,
    colorNeutralBackground3Pressed: u.pressed,
    colorNeutralBackground3Selected: u.selected,
    colorNeutralBackground4: f.navigation,
    colorNeutralBackground4Hover: n.hover,
    colorNeutralBackground4Pressed: n.pressed,
    colorNeutralBackground4Selected: n.selected,
    colorNeutralCardBackground: f.karte,
    colorNeutralCardBackgroundHover: k.hover,
    colorNeutralCardBackgroundPressed: k.pressed,
    colorNeutralCardBackgroundSelected: k.selected,
  };
}

function radiusTokens(radius: AppDesign["radius"]): Partial<Theme> {
  if (radius === "eckig") {
    return {
      borderRadiusSmall: "0",
      borderRadiusMedium: "0",
      borderRadiusLarge: "0",
      borderRadiusXLarge: "0",
    };
  }
  if (radius === "rund") {
    return {
      borderRadiusSmall: "4px",
      borderRadiusMedium: "8px",
      borderRadiusLarge: "12px",
      borderRadiusXLarge: "16px",
    };
  }
  return {};
}

/**
 * Entspricht das Design dem eingebauten Microsoft-Design (Identitaet oder dieselbe Kennung ohne eigene Werte)?
 * Nur dann gilt die Abkuerzung auf die Fluent-Originalthemes; ein App-Design, das „microsoft“ ersetzt, behaelt
 * seine Marke, Flaechen, Radien und Schrift.
 */
export function istFluentOriginal(design: AppDesign): boolean {
  if (design === MICROSOFT) return true;
  return (
    design.id === MICROSOFT.id &&
    design.marke.hell.toUpperCase() === MICROSOFT.marke.hell &&
    design.marke.dunkel.toUpperCase() === MICROSOFT.marke.dunkel &&
    !design.flaechen &&
    !design.schrift &&
    !design.markeAlsFlaecheDunkel &&
    (design.radius ?? "standard") === "standard"
  );
}

/** Zustaende der Markenflaeche, auf denen `colorNeutralForegroundOnBrand` steht. */
export const MARKENFLAECHEN_ZUSTAENDE = [
  "colorBrandBackground",
  "colorBrandBackgroundHover",
  "colorBrandBackgroundPressed",
  "colorBrandBackgroundSelected",
] as const;

/** Fluent-Theme fuer ein Design. Reine Funktion; das Ergebnis ist fuer `FluentProvider theme` gedacht. */
export function themeFuerDesign(
  design: AppDesign,
  schema: Schema,
  optionen: ThemeOptionen = {},
): Theme {
  if (optionen.kontrast === "hoch") return teamsHighContrastTheme;
  const dunkel = schema === "dunkel";
  const ohneAkzent = !istHex(optionen.akzent) && !akzentBasis(optionen.akzent);
  if (ohneAkzent && istFluentOriginal(design)) return dunkel ? webDarkTheme : webLightTheme;

  const marke = wirksameMarke(design, schema, optionen.akzent);
  const ziel = dunkel ? WEISS : SCHWARZ;
  const markeHover = mische(marke, ziel, 0.18);
  const rampe = brandRampe(marke, dunkel ? 100 : 80) as BrandVariants;
  const basis = dunkel ? createDarkTheme(rampe) : createLightTheme(rampe);

  // Markenflaeche (primaere Knoepfe, Auswahl): hell Stufe 80, dunkel Stufe 70 oder die helle Marke selbst.
  const alsFlaeche = dunkel && design.markeAlsFlaecheDunkel === true;
  const flaeche = alsFlaeche ? marke : basis.colorBrandBackground;
  const flaecheTokens: Partial<Theme> = alsFlaeche
    ? {
        colorBrandBackground: marke,
        colorBrandBackgroundHover: mische(marke, WEISS, 0.3),
        colorBrandBackgroundPressed: mische(marke, SCHWARZ, 0.12),
        colorBrandBackgroundSelected: mische(marke, WEISS, 0.15),
        colorCompoundBrandBackground: marke,
        colorCompoundBrandBackgroundHover: mische(marke, WEISS, 0.3),
        colorCompoundBrandBackgroundPressed: mische(marke, SCHWARZ, 0.12),
        colorCompoundBrandStroke: marke,
        colorCompoundBrandStrokeHover: mische(marke, WEISS, 0.3),
        colorBrandStroke1: marke,
        colorStrokeFocus2: marke,
      }
    : {};

  const theme: Theme = {
    ...basis,
    ...(design.flaechen ? flaechenTokens(design.flaechen[schema], schema) : {}),
    ...radiusTokens(design.radius),
    ...(design.schrift ? { fontFamilyBase: design.schrift } : {}),
    // Markentext und Links: immer die kontrastgepruefte Marke.
    colorBrandForeground1: marke,
    colorBrandForeground2: markeHover,
    colorBrandForegroundLink: marke,
    colorBrandForegroundLinkHover: markeHover,
    colorBrandForegroundLinkPressed: markeHover,
    colorBrandForegroundLinkSelected: marke,
    colorCompoundBrandForeground1: marke,
    colorCompoundBrandForeground1Hover: markeHover,
    colorCompoundBrandForeground1Pressed: markeHover,
    ...flaecheTokens,
  };
  // Schrift auf der Markenflaeche: Weiss oder Tinte, je nachdem, was besser lesbar ist. Jeder Zustand der
  // Flaeche (Ruhe, Hover, gedrueckt, ausgewaehlt) wird bei Bedarf von der Schrift weg verschoben, bis 4,5:1 gilt.
  const schrift = textAuf(flaeche);
  const weg = schrift === WEISS ? SCHWARZ : WEISS;
  for (const k of MARKENFLAECHEN_ZUSTAENDE) {
    const farbe = theme[k];
    if (istHex(farbe))
      theme[k] = verschiebeBisKontrast(farbe, weg, [schrift], KONTRAST_TEXT + 0.05);
  }
  theme.colorNeutralForegroundOnBrand = schrift;
  return theme;
}

/** Bewegung aus: alle Fluent-Dauern auf 0 ms (Animationen-Schalter, reduzierte Bewegung). */
export function ohneBewegung(theme: Theme): Theme {
  return {
    ...theme,
    durationUltraFast: "0ms",
    durationFaster: "0ms",
    durationFast: "0ms",
    durationNormal: "0ms",
    durationGentle: "0ms",
    durationSlow: "0ms",
    durationSlower: "0ms",
    durationUltraSlow: "0ms",
  };
}
