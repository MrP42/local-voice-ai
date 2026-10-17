import type { Theme } from "@fluentui/react-components";
import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
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
  loadAppearance,
  persistAppearance,
  type Saved,
  serializeAppearance,
  themeFromModus,
  uebernehmen,
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

  // Abgleich mit AppSettings (führend): nach dem Laden direkt in den Hook
  // übernehmen, erst danach speichern. Kommt die Antwort spät, gilt sie
  // trotzdem — es gibt kein Ereignis, das verloren gehen könnte.
  const zRef = useRef(z);
  zRef.current = z;
  // Für den Hinweis nach dem letzten Ladeversuch; ein Sprachwechsel startet den Abgleich nicht neu.
  const tRef = useRef(t);
  tRef.current = t;
  const savedRef = useRef<Saved | null>(null);
  const [synced, setSynced] = useState(false);
  // Scheitert das Laden, wird es mit wachsendem Abstand wiederholt; erst nach
  // dem letzten Versuch erscheint ein Hinweis. Ohne Abgleich wird nicht
  // gespeichert, damit der Backend-Stand nicht blind überschrieben wird.
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const pausen = [1000, 3000, 10000, 30000];
    const versuch = (n: number) => {
      const { modus, kontrast, animationen, designId, akzent } = zRef.current;
      void loadAppearance({
        modus,
        kontrast,
        animationen,
        designId,
        akzent,
      }).then((loaded) => {
        if (cancelled) return;
        if (!loaded) {
          if (n < pausen.length)
            timer = setTimeout(() => versuch(n + 1), pausen[n]);
          else toast.error(tRef.current("appearance.saveFailed"));
          return;
        }
        uebernehmen(zRef.current, loaded.stand);
        savedRef.current = {
          appearance: loaded.migrated
            ? null
            : serializeAppearance(loaded.stand),
          theme: loaded.theme,
        };
        setSynced(true);
      });
    };
    versuch(0);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  // Speichern: nur Abweichungen vom nachweislich gespeicherten Stand, der
  // Reihe nach. Fehlgeschlagenes bleibt abweichend und wird beim nächsten
  // Wechsel erneut geschrieben.
  const json = serializeAppearance(z);
  const chain = useRef<Promise<void>>(Promise.resolve());
  useEffect(() => {
    if (!synced) return;
    chain.current = chain.current.then(async () => {
      if (!savedRef.current) return;
      const r = await persistAppearance(JSON.parse(json), savedRef.current);
      savedRef.current = r.saved;
      if (!r.ok) toast.error(t("appearance.saveFailed"));
    });
  }, [synced, json, t]);

  return (
    <ErscheinungsbildProvider zustand={z} className="lv-fluent-root">
      {children}
    </ErscheinungsbildProvider>
  );
}
