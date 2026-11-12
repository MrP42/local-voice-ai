/**
 * Der eine Store für `state.json` im laufenden Programm (lva#530 I1, Codex
 * Runde 5 Nachtrag): Vorlesen (TtsSettings), Bibliothek (Anhängen, Zusammenführen,
 * Fassungen) und „Sprecher ersetzen“ schreiben alle über DIESELBE Kette, damit
 * kein fremder Schreiber eine gesicherte Rohfassung (`einfuegungen`) mit einem
 * älteren Stand überschreibt.
 *
 * Ein-/Ausgabe geht über Tauri, die Sicht auf die offene Seite hängt das
 * Vorlesen ein (`bindPageStateLive`). Ohne Vorlesen ist keine Seite offen.
 * Die Logik selbst steht in `pageState.ts` (ohne Tauri, per Node getestet).
 */
import { commands } from "@/bindings";
import {
  createPageStateStore,
  type PageStateLive,
  type PageStateStore,
} from "./pageState";
import type { PasteLogEntry } from "./pasteLog";

const NO_VIEW: PageStateLive = {
  isLive: () => false,
  build: () => {
    throw new Error("keine offene Seite");
  },
};

let view: PageStateLive = NO_VIEW;
let commitListener: ((page: string, entries: PasteLogEntry[]) => void) | null =
  null;
let store: PageStateStore | null = null;

/** Der Store des Programms; wird beim ersten Zugriff angelegt. */
export function getPageStateStore(): PageStateStore {
  store ??= createPageStateStore({
    io: {
      load: async (page) => {
        const loaded = await commands.pageStateLoad(page).catch(() => null);
        return loaded && loaded.status === "ok" ? (loaded.data ?? "") : null;
      },
      save: async (page, json) => {
        const saved = await commands
          .pageStateSave(page, json)
          .catch(() => null);
        return saved !== null && saved.status === "ok";
      },
    },
    live: {
      isLive: (page) => view.isLive(page),
      build: () => view.build(),
    },
    onCommit: (page, entries) => commitListener?.(page, entries),
  });
  return store;
}

/**
 * Das Vorlesen meldet die offene Seite an. Gibt die Abmeldung zurück (nur
 * die eigene Anmeldung wird gelöst, eine neuere bleibt bestehen).
 */
export function bindPageStateLive(
  next: PageStateLive,
  onCommit: (page: string, entries: PasteLogEntry[]) => void,
): () => void {
  view = next;
  commitListener = onCommit;
  return () => {
    if (view !== next) return;
    view = NO_VIEW;
    commitListener = null;
  };
}
