import { createContext, useContext } from "react";

/**
 * Andockplatz der Seitenhilfe am rechten Fensterrand (Suite-Standard): Die
 * Hilfe steht neben der Inhaltsschicht, nicht in der zentrierten Spalte, damit
 * der Inhalt seine Breite behält. Ohne Platz (Tests) bleibt sie im Inhalt.
 */
export const HelpDockSlotContext = createContext<HTMLElement | null>(null);

export const useHelpDockSlot = () => useContext(HelpDockSlotContext);
