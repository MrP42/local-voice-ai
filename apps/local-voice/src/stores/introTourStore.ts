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
  /** Automatischer Download gescheitert: App zeigt die Modellauswahl. */
  setupFailed: boolean;
  start: () => void;
  /** Erster Start: nur, wenn die Tour auf diesem Gerät noch nie lief
      (ein abgebrochener Download wiederholt sonst die Tour). */
  startFirstTime: () => void;
  close: () => void;
  setPendingModel: (id: string | null) => void;
  setSetupFailed: (failed: boolean) => void;
}

const SEEN_KEY = "lva.introTour.seen";

const seen = () => {
  try {
    return localStorage.getItem(SEEN_KEY) === "1";
  } catch {
    return false;
  }
};

const markSeen = () => {
  try {
    localStorage.setItem(SEEN_KEY, "1");
  } catch {
    /* Ohne Speicher erscheint die Tour beim nächsten Erststart erneut. */
  }
};

export const useIntroTourStore = create<IntroTourState>()((set) => ({
  open: false,
  pendingModelId: null,
  setupFailed: false,
  start: () => set({ open: true }),
  startFirstTime: () => {
    if (!seen()) set({ open: true });
  },
  close: () => {
    markSeen();
    set({ open: false });
  },
  setPendingModel: (pendingModelId) => set({ pendingModelId }),
  setSetupFailed: (setupFailed) => set({ setupFailed }),
}));
