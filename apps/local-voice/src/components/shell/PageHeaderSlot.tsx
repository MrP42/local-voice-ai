import { createContext, useContext } from "react";

/**
 * Platz für den Seitenkopf in der Kopfleiste (Leitlinie §2: Seitentitel in der
 * Kopfleiste, nur der Inhalt scrollt). PageShell rendert Titel und Aktionen per
 * Portal hierher; ohne Platz (Tests, Onboarding) bleibt der Kopf im Inhalt.
 */
export const PageHeaderSlotContext = createContext<HTMLElement | null>(null);

export const usePageHeaderSlot = () => useContext(PageHeaderSlotContext);

/**
 * Andockplatz der Seitenhilfe am rechten Fensterrand (Suite-Standard): Die
 * Hilfe steht neben der Inhaltsschicht, nicht in der zentrierten Spalte, damit
 * der Inhalt seine Breite behält. Ohne Platz (Tests) bleibt sie im Inhalt.
 */
export const HelpDockSlotContext = createContext<HTMLElement | null>(null);

export const useHelpDockSlot = () => useContext(HelpDockSlotContext);
