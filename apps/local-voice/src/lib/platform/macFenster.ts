import { isTauri } from "@tauri-apps/api/core";
import { commands } from "@/bindings";

/**
 * Fensterverhalten auf dem Mac (lva#438). Das Hauptfenster hat dort die
 * nativen Ampelknöpfe über der eigenen Titelleiste (lib.rs), die
 * Windows-Fensterknöpfe entfallen.
 *
 * - Doppelklick auf die freie Titelleiste folgt der Systemeinstellung
 *   (Rust-Befehl `titelleiste_doppelklick`) statt immer zu maximieren.
 * - Echtes macOS-Vollbild: `setzeMacVollbild`, und der Zustand steht als
 *   `data-mac-vollbild` am Wurzelelement (dann ohne Platz für die Ampel).
 *
 * Auf allen anderen Systemen tut dieses Modul nichts.
 */
export function istMac(): boolean {
  // main.tsx setzt das Attribut vor dem ersten Rendern (plugin-os).
  return document.documentElement.dataset.platform === "macos";
}

export const MAC_VOLLBILD_EVENT = "lv-mac-vollbild";

async function fenster() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export async function istMacVollbild(): Promise<boolean> {
  if (!istMac() || !isTauri()) return false;
  try {
    return await (await fenster()).isFullscreen();
  } catch {
    return false;
  }
}

/** Schaltet das native Vollbild des Hauptfensters (nur Mac). */
export async function setzeMacVollbild(an: boolean): Promise<void> {
  if (!istMac() || !isTauri()) return;
  try {
    await (await fenster()).setFullscreen(an);
  } catch {
    /* Fenster nicht erreichbar: Lese-Vollbild bleibt im Fenster */
  }
}

const KLICKBAR = new Set([
  "A",
  "BUTTON",
  "INPUT",
  "SELECT",
  "TEXTAREA",
  "LABEL",
  "SUMMARY",
]);
const ROLLEN = new Set([
  "button",
  "link",
  "menuitem",
  "tab",
  "checkbox",
  "radio",
  "switch",
  "option",
]);

/** Dieselbe Regel wie Tauris drag.js: liegt der Klick auf einer Ziehfläche? */
export function istZiehflaeche(pfad: EventTarget[]): boolean {
  for (const el of pfad) {
    if (!(el instanceof HTMLElement)) continue;
    const attr = el.getAttribute("data-tauri-drag-region");
    const klickbar =
      KLICKBAR.has(el.tagName) ||
      (el.hasAttribute("contenteditable") &&
        el.getAttribute("contenteditable") !== "false") ||
      (el.hasAttribute("tabindex") && el.getAttribute("tabindex") !== "-1") ||
      ROLLEN.has(el.getAttribute("role") ?? "");
    if (klickbar && attr === null) return false;
    if (attr === null) continue;
    if (attr === "false") return false;
    if (attr === "deep") return true;
    if (attr === "" || attr === "true") return el === pfad[0];
  }
  return false;
}

let eingerichtet = false;

/**
 * Einmal beim Start aufrufen. Fängt auf dem Mac den Doppelklick auf
 * Ziehflächen ab, bevor Tauris drag.js (auf `document`) ihn als „immer
 * maximieren“ ausführt, und meldet Vollbildwechsel.
 */
export function richteMacFensterEin(): void {
  if (eingerichtet || !istMac() || !isTauri()) return;
  eingerichtet = true;

  // Wie drag.js: Aktion erst beim Loslassen und nur ohne Mausbewegung.
  let startX = 0;
  let startY = 0;
  window.addEventListener(
    "mousedown",
    (e) => {
      if (e.button === 0 && e.detail === 2) {
        startX = e.clientX;
        startY = e.clientY;
      }
    },
    true,
  );
  window.addEventListener(
    "mouseup",
    (e) => {
      if (e.button !== 0 || e.detail !== 2) return;
      if (!istZiehflaeche(e.composedPath())) return;
      // Erfassungsphase am Fenster: drag.js am Dokument sieht ihn nicht mehr.
      e.stopImmediatePropagation();
      if (e.clientX !== startX || e.clientY !== startY) return;
      void commands.titelleisteDoppelklick().catch(() => undefined);
    },
    true,
  );

  const melde = (an: boolean) => {
    const wurzel = document.documentElement;
    if (an) wurzel.dataset.macVollbild = "1";
    else delete wurzel.dataset.macVollbild;
    window.dispatchEvent(
      new CustomEvent<boolean>(MAC_VOLLBILD_EVENT, { detail: an }),
    );
  };
  void (async () => {
    try {
      const w = await fenster();
      let zuletzt = await w.isFullscreen();
      melde(zuletzt);
      // Beim Ziehen am Rand kommen viele Ereignisse: erst nach Ruhe prüfen.
      let warte: ReturnType<typeof setTimeout> | undefined;
      await w.onResized(() => {
        clearTimeout(warte);
        warte = setTimeout(() => {
          void w
            .isFullscreen()
            .catch(() => zuletzt)
            .then((an) => {
              if (an === zuletzt) return;
              zuletzt = an;
              melde(an);
            });
        }, 120);
      });
    } catch {
      /* Zustand nicht lesbar: Titelleiste behält Platz für die Ampel */
    }
  })();
}
