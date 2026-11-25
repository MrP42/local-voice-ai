import { create } from "zustand";

/**
 * Offener Projektraum im Chat (lva#826): Ein Klick auf ein Projekt in der Seitenleiste des Chats zeigt
 * statt des Gesprächs die Startansicht des Projekts (Frage im Projekt, Gespräche, Quellen). Das Öffnen
 * eines Gesprächs schließt den Raum wieder. Nur Anzeige, keine Freigabe: was ein Projektgespräch sehen
 * darf, entscheidet weiter das Rust-Tor.
 */
interface ProjektRaumState {
  raumId: string | null;
  oeffnen: (projektId: string) => void;
  schliessen: () => void;
}

export const useProjektRaum = create<ProjektRaumState>((set) => ({
  raumId: null,
  oeffnen: (projektId) => set({ raumId: projektId }),
  schliessen: () => set({ raumId: null }),
}));
