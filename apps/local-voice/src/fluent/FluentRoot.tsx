import {
  FluentProvider,
  webDarkTheme,
  webLightTheme,
  type Theme,
} from "@fluentui/react-components";
import { type ReactNode, useLayoutEffect, useSyncExternalStore } from "react";

/**
 * Fluent 2 an der Wurzel (Leitlinie windows-fluent.md §1, §10).
 *
 * Palette: Microsoft (webLightTheme/webDarkTheme). Hell/Dunkel folgt der
 * bestehenden App-Einstellung: `html[data-theme]` (lib/utils/theme.ts) gewinnt,
 * sonst das System (`prefers-color-scheme`).
 *
 * Die Fluent-Tokens stehen zusätzlich als CSS-Variablen auf `:root`. So zieht
 * die Variablenbrücke (styles/fluent-bridge.css) auch Flächen außerhalb des
 * Providers mit: `body`, eigene Portale (Tooltips, Menüs, react-select) und
 * das Overlay-Fenster.
 */

export type ColorScheme = "light" | "dark";

const DARK_QUERY = "(prefers-color-scheme: dark)";

function subscribe(cb: () => void) {
  const mq = window.matchMedia?.(DARK_QUERY);
  mq?.addEventListener?.("change", cb);
  const mo = new MutationObserver(cb);
  mo.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  return () => {
    mq?.removeEventListener?.("change", cb);
    mo.disconnect();
  };
}

function readScheme(): ColorScheme {
  const attr = document.documentElement.getAttribute("data-theme");
  if (attr === "light" || attr === "dark") return attr;
  return window.matchMedia?.(DARK_QUERY).matches ? "dark" : "light";
}

export function useColorScheme(): ColorScheme {
  return useSyncExternalStore(subscribe, readScheme, () => "light" as const);
}

export function themeFor(scheme: ColorScheme): Theme {
  return scheme === "dark" ? webDarkTheme : webLightTheme;
}

/** Schreibt alle Tokens des Themes als `--<token>` auf `:root`. */
export function applyThemeTokens(theme: Theme, scheme: ColorScheme) {
  const root = document.documentElement;
  for (const [key, value] of Object.entries(theme)) {
    root.style.setProperty(`--${key}`, String(value));
  }
  root.dataset.fluent = "";
  root.dataset.scheme = scheme;
  root.style.colorScheme = scheme;
}

export function FluentRoot({ children }: { children: ReactNode }) {
  const scheme = useColorScheme();
  const theme = themeFor(scheme);
  useLayoutEffect(() => applyThemeTokens(theme, scheme), [theme, scheme]);
  return (
    <FluentProvider
      theme={theme}
      className="lv-fluent-root"
      data-scheme={scheme}
      style={{ colorScheme: scheme }}
    >
      {children}
    </FluentProvider>
  );
}
