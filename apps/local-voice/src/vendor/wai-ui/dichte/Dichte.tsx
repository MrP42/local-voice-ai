import { createContext, useContext } from "react";

export type Dichte = "kompakt" | "normal";

/** Nur der Kontext (ohne DOM); `WaiProvider` setzt damit die Dichte und `data-density` am eigenen Wurzelelement. */
export const DichteKontext = createContext<Dichte>("normal");

/** Wirksame Dichte des nächsten `DichteProvider` bzw. `WaiProvider`; ohne Provider „normal“. */
export function useDichte(): Dichte {
  return useContext(DichteKontext);
}

