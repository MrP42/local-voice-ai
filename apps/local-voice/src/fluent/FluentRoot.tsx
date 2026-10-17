import type { Theme } from "@fluentui/react-components";
import { type ReactNode, useEffect, useLayoutEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  type AppDesign,
  ErscheinungsbildProvider,
  MICROSOFT,
  useErscheinungsbild,
  WAI,
} from "@/erscheinungsbild";
import { applyTheme } from "@/lib/utils/theme";
import {
  APPEARANCE_STORAGE_KEY,
  persistAppearance,
  themeFromModus,
} from "./appearance";

/**
 * Fluent 2 an der Wurzel (Leitlinie windows-fluent.md §1, §10) mit dem
 * SDK-Baustein „Erscheinungsbild“: Design (Fluent als Standard, WAI),
 * Akzent, Farbschema, Kontrast und Animationen bestimmen das Theme
 * (`themeFuerDesign`).
 *
 * Die Tokens des wirksamen Themes stehen zusätzlich als CSS-Variablen auf
 * `:root`. So zieht die Variablenbrücke (styles/fluent-bridge.css) auch
 * Flächen außerhalb des Providers mit — `body`, eigene Portale (Tooltips,
 * Menüs, react-select) — und folgt jedem Design- und Akzentwechsel zur
 * Laufzeit.
 */

export type ColorScheme = "light" | "dark";

/** Schreibt alle Tokens des Themes als `--<token>` auf `:root`. */
export function applyThemeTokens(
  theme: Theme,
  scheme: ColorScheme,
  design: string,
) {
  const root = document.documentElement;
  for (const [key, value] of Object.entries(theme)) {
    root.style.setProperty(`--${key}`, String(value));
  }
  root.dataset.fluent = "";
  root.dataset.scheme = scheme;
  root.dataset.design = design;
  root.style.colorScheme = scheme;
}

export function FluentRoot({ children }: { children: ReactNode }) {
  const { t, i18n } = useTranslation();
  // Anzeigetexte der Designs aus der App-i18n; gleiche Kennung ersetzt das
  // eingebaute Design (Werte unverändert, Fluent bleibt webLight/webDark).
  const designs = useMemo<AppDesign[]>(
    () => [
      {
        ...MICROSOFT,
        beschreibung: t("appearance.designs.fluent", {
          defaultValue: MICROSOFT.beschreibung,
        }),
      },
      {
        ...WAI,
        beschreibung: t("appearance.designs.wai", {
          defaultValue: WAI.beschreibung,
        }),
      },
    ],
    // `t` wechselt mit der Sprache; die Liste bleibt sonst stabil.
    [t, i18n.language],
  );
  const z = useErscheinungsbild({
    speicherSchluessel: APPEARANCE_STORAGE_KEY,
    designs,
  });
  const scheme: ColorScheme = z.schema === "dunkel" ? "dark" : "light";

  useLayoutEffect(() => {
    applyThemeTokens(z.theme, scheme, z.design.id);
  }, [z.theme, scheme, z.design.id]);

  // Altes Attribut `data-theme` und `handy.theme` als Spiegel des
  // Farbschemas (App.css-Palette, Meeting-Hinweisfenster).
  useLayoutEffect(() => {
    applyTheme(themeFromModus(z.modus));
  }, [z.modus]);

  const { modus, kontrast, animationen, designId, akzent } = z;
  useEffect(() => {
    void persistAppearance({ modus, kontrast, animationen, designId, akzent });
  }, [modus, kontrast, animationen, designId, akzent]);

  return (
    <ErscheinungsbildProvider zustand={z} className="lv-fluent-root">
      {children}
    </ErscheinungsbildProvider>
  );
}
