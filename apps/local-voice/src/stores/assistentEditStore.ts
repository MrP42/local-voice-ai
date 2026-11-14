/**
 * Was die Seite „Assistenten“ gerade offen hat (lva#556, G2 Teil 2): der geöffnete Eintrag und der Entwurf eines eigenen
 * Assistenten. Der Entwurf liegt hier und nicht im Zustand der Seite, damit ein Wechsel in einen anderen Bereich der App ihn
 * nicht verwirft: wer zurückkommt, findet seine ungespeicherten Änderungen wieder. Geschrieben wird erst mit „Speichern“.
 */
import { create } from "zustand";
import type { Profil } from "@/components/voice-call/assistenten";

/** Ein Eintrag der Liste: Vorlage, eigener Assistent oder eine Datei, die der Speicher abgewiesen hat. */
export type Eintrag =
  | { art: "vorlage"; id: string }
  | { art: "eigen"; id: string }
  | { art: "abgewiesen"; datei: string };

/** Der Entwurf eines eigenen Assistenten und der Stand der Datei, auf dem er begann. */
export interface Entwurf {
  id: string;
  /** Der gespeicherte Stand, auf dem der Entwurf begann (Profil und Text der Datei). */
  basis: { profil: Profil; text: string };
  profil: Profil;
  /** Die Datei hat sich unter dem Entwurf geändert (oder es gibt sie nicht mehr): nichts wird still überschrieben. */
  konflikt: "geaendert" | "fehlt" | null;
}

export interface AssistentEditStore {
  offen: Eintrag | null;
  entwurf: Entwurf | null;
  oeffne: (eintrag: Eintrag | null) => void;
  setzeEntwurf: (entwurf: Entwurf | null) => void;
  aendere: (aenderung: (p: Profil) => Profil) => void;
  setzeKonflikt: (konflikt: Entwurf["konflikt"]) => void;
}

export const useAssistentEditStore = create<AssistentEditStore>()((set) => ({
  offen: null,
  entwurf: null,
  oeffne: (offen) => set({ offen }),
  setzeEntwurf: (entwurf) => set({ entwurf }),
  aendere: (aenderung) =>
    set((s) =>
      s.entwurf
        ? { entwurf: { ...s.entwurf, profil: aenderung(s.entwurf.profil) } }
        : s,
    ),
  setzeKonflikt: (konflikt) =>
    set((s) => (s.entwurf ? { entwurf: { ...s.entwurf, konflikt } } : s)),
}));
