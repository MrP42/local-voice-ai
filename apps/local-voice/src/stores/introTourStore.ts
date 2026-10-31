import { create } from "zustand";

/**
 * Einführungstour und automatisch eingerichtetes Diktatmodell beim ersten
 * Start. Die Tour öffnet die Erstinstallation; danach jederzeit über die
 * Startseite oder die Hilfe einer Seite.
 */
interface IntroTourState {
  open: boolean;
  /** Modell, das der erste Start gerade lädt und danach auswählt. */
  pendingModelId: string | null;
  start: () => void;
  close: () => void;
  setPendingModel: (id: string | null) => void;
}

export const useIntroTourStore = create<IntroTourState>()((set) => ({
  open: false,
  pendingModelId: null,
  start: () => set({ open: true }),
  close: () => set({ open: false }),
  setPendingModel: (pendingModelId) => set({ pendingModelId }),
}));
