import { isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * Fensterknöpfe wie Windows 11 (Minimieren, Maximieren/Wiederherstellen,
 * Schließen) für das rahmenlose Hauptfenster. Nur in der Desktop-App (Tauri
 * setzt `window.isTauri`); im Browser und in Tests ohne Tauri rendert nichts.
 *
 * „Schließen“ ruft `close()` auf und löst damit dasselbe `CloseRequested`
 * aus wie das native X: lib.rs verhindert das Schließen und blendet das
 * Fenster aus (die App läuft im Infobereich weiter).
 */
async function currentWindow() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

type AppWindow = Awaited<ReturnType<typeof currentWindow>>;

/** Glyphen nach Segoe-Fluent-Vorbild: 10 px, 1 px Linie. */
function Glyph({ kind }: { kind: "min" | "max" | "restore" | "close" }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      aria-hidden="true"
      focusable="false"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
    >
      {kind === "min" ? <path d="M0 5.5h10" /> : null}
      {kind === "max" ? (
        <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
      ) : null}
      {kind === "restore" ? (
        <>
          <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
          <path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
        </>
      ) : null}
      {kind === "close" ? <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" /> : null}
    </svg>
  );
}

export function WindowControls() {
  const { t } = useTranslation();
  const [active] = useState(() => isTauri());
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!active) return;
    let unlisten: (() => void) | undefined;
    let gone = false;
    void (async () => {
      try {
        const w = await currentWindow();
        const check = async () => setMaximized(await w.isMaximized());
        await check();
        const off = await w.onResized(() => void check());
        if (gone) off();
        else unlisten = off;
      } catch {
        /* Fensterzustand nicht lesbar: Knopf zeigt „Maximieren“ */
      }
    })();
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [active]);

  if (!active) return null;
  const run = (f: (w: AppWindow) => Promise<void>) => () =>
    void currentWindow()
      .then(f)
      .catch(() => undefined);
  const minimize = t("workspace.windowMinimize");
  const toggle = maximized
    ? t("workspace.windowRestore")
    : t("workspace.windowMaximize");
  const close = t("workspace.windowClose");
  return (
    // Bewusst ohne Tooltip-Komponente: Windows zeigt an den Fensterknöpfen nur
    // den nativen Hinweis.
    <div
      className="lv-window-controls"
      role="group"
      aria-label={t("workspace.windowControls")}
    >
      <button
        type="button"
        className="lv-window-button"
        aria-label={minimize}
        title={minimize}
        onClick={run((w) => w.minimize())}
      >
        <Glyph kind="min" />
      </button>
      <button
        type="button"
        className="lv-window-button"
        aria-label={toggle}
        title={toggle}
        onClick={run((w) => w.toggleMaximize())}
      >
        <Glyph kind={maximized ? "restore" : "max"} />
      </button>
      <button
        type="button"
        className="lv-window-button lv-window-button--close"
        aria-label={close}
        title={close}
        onClick={run((w) => w.close())}
      >
        <Glyph kind="close" />
      </button>
    </div>
  );
}
