import { create } from "zustand";

/**
 * Offener Projektraum im Chat (lva#826): Ein Klick auf ein Projekt in der Seitenleiste des Chats zeigt
 * statt des Gesprächs die Startansicht des Projekts (Frage im Projekt, Gespräche, Quellen). Das Öffnen
 * eines Gesprächs schließt den Raum wieder. Nur Anzeige, keine Freigabe: was ein Projektgespräch sehen
 * darf, entscheidet weiter das Rust-Tor.
 */
interface ProjektRaumState {
  raumId: string | null;
  /**
   * Zählt jedes Öffnen und Schließen. Eine laufende erste Frage im Raum prüft ihn vor dem Anlegen des
   * Gesprächs: hat die Person inzwischen woanders hingeklickt, legt sie nichts mehr an.
   */
  gen: number;
  oeffnen: (projektId: string) => void;
  schliessen: () => void;
}

export const useProjektRaum = create<ProjektRaumState>((set) => ({
  raumId: null,
  gen: 0,
  oeffnen: (projektId) => set((s) => ({ raumId: projektId, gen: s.gen + 1 })),
  schliessen: () =>
    set((s) => (s.raumId === null ? s : { raumId: null, gen: s.gen + 1 })),
}));
