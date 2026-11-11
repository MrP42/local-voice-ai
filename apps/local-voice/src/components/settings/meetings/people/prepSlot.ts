import { createContext } from "react";

/**
 * Der Platz am unteren Rand der Notizfläche, an dem die Vorbereitungskarte
 * schwebt (G4, #499). Sie liegt über der Fläche statt in ihrem Fluss: weder ihr
 * Eintreffen noch das Einklappen verschiebt etwas, was darüber steht (die
 * Schreibfläche). Ohne Platz (null) rendert die Karte im Fluss.
 */
export const PrepSlotContext = createContext<HTMLElement | null>(null);
