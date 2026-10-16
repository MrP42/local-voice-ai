import { createContext, useContext } from "react";

/**
 * Platz für den Seitenkopf in der Kopfleiste (Leitlinie §2: Seitentitel in der
 * Kopfleiste, nur der Inhalt scrollt). PageShell rendert Titel und Aktionen per
 * Portal hierher; ohne Platz (Tests, Onboarding) bleibt der Kopf im Inhalt.
 */
export const PageHeaderSlotContext = createContext<HTMLElement | null>(null);

export const usePageHeaderSlot = () => useContext(PageHeaderSlotContext);
