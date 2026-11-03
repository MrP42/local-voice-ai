import { create } from "zustand";

/**
 * Fußleiste je Modul (Patrick 02.11.): Die Modelle, die das geöffnete Modul
 * gerade nutzt, stehen immer links unten in der Statusleiste — nicht mehr im
 * Profilmenü und nicht mitten in der Seite. Ein Modul kann zusätzlich eigene
 * Bedienelemente (z. B. die Stimme beim Vorlesen) per Portal in den Platz
 * `modulPlatz` hängen.
 *
 * Daneben ein kurzer Status („Skript geprüft: keine Befunde.“), der von selbst
 * wieder verschwindet und keinen Inhalt verschiebt.
 */
interface FussleisteState {
  /** Ziel für Portale der Module (links in der Statusleiste). */
  modulPlatz: HTMLElement | null;
  setModulPlatz: (el: HTMLElement | null) => void;
  /** Aktueller Kurzstatus; `null` = nichts anzeigen. */
  status: { text: string; id: number } | null;
  zeigeStatus: (text: string, ms?: number) => void;
  loescheStatus: () => void;
}

/** Standarddauer eines Kurzstatus in Millisekunden. */
export const STATUS_DAUER_MS = 4000;

let timer: ReturnType<typeof setTimeout> | null = null;
let naechsteId = 1;

export const useFussleisteStore = create<FussleisteState>((set) => ({
  modulPlatz: null,
  setModulPlatz: (el) => set({ modulPlatz: el }),
  status: null,
  zeigeStatus: (text, ms = STATUS_DAUER_MS) => {
    if (timer) clearTimeout(timer);
    const id = naechsteId++;
    set({ status: { text, id } });
    timer = setTimeout(() => {
      timer = null;
      set((s) => (s.status?.id === id ? { status: null } : s));
    }, ms);
  },
  loescheStatus: () => {
    if (timer) clearTimeout(timer);
    timer = null;
    set({ status: null });
  },
}));
