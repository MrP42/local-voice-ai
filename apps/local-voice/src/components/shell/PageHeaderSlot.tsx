import { createContext, useContext } from "react";

/**
 * Andockplatz der Seitenhilfe am rechten Fensterrand (Suite-Standard): Die
 * Hilfe steht neben der Inhaltsschicht, nicht in der zentrierten Spalte, damit
 * der Inhalt seine Breite behält. Ohne Platz (Tests) bleibt sie im Inhalt.
 */
export const HelpDockSlotContext = createContext<HTMLElement | null>(null);

export const useHelpDockSlot = () => useContext(HelpDockSlotContext);

/**
 * Platz für die Bedienelemente des Moduls (ⓘ, Suche, Aktionen) rechts in der
 * Titelleiste. Das Modul hat keine eigene Titelzeile mehr (Patrick 02.11.):
 * der Name steht in der Navigation links, der Inhalt bekommt die Höhe. Ohne
 * Platz (Tests ohne App-Rahmen) bleiben die Bedienelemente als schmale Zeile
 * im Inhalt.
 */
export const PageHeaderSlotContext = createContext<HTMLElement | null>(null);

export const usePageHeaderSlot = () => useContext(PageHeaderSlotContext);
