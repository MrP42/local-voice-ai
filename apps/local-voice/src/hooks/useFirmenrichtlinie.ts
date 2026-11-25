import { useEffect, useState } from "react";
import { commands, type RichtlinienAusgabe } from "@/bindings";
import { istGesperrt, type Schalter } from "../lib/firmenrichtlinie";

// Die Richtlinie wird beim Start der App einmal gelesen und ändert sich danach
// nicht mehr (lva#882). Eine Abfrage je Fenster genügt.
let anfrage: Promise<RichtlinienAusgabe | null> | null = null;
let stand: RichtlinienAusgabe | null = null;

const laden = (): Promise<RichtlinienAusgabe | null> => {
  anfrage ??= commands
    .richtlinieLesen()
    .then((ausgabe) => {
      stand = ausgabe;
      return ausgabe;
    })
    .catch((fehler: unknown) => {
      // Die Anzeige ist nur Erklärung: Durchgesetzt wird im Backend, das gesperrte
      // Aktionen mit `richtlinie:<schalter>:` ablehnt (Meldung über richtlinienText).
      // Scheitert das Lesen, bleiben die Schalter bedienbar, und der nächste Aufruf
      // versucht es erneut.
      console.warn("Firmenrichtlinie nicht lesbar", fehler);
      anfrage = null;
      return null;
    });
  return anfrage;
};

export interface Firmenrichtlinie {
  ausgabe: RichtlinienAusgabe | null;
  gesperrt: (schalter: Schalter) => boolean;
}

/** Wirksame Firmenrichtlinie; vor dem ersten Laden gilt nichts als gesperrt. */
export const useFirmenrichtlinie = (): Firmenrichtlinie => {
  const [ausgabe, setAusgabe] = useState<RichtlinienAusgabe | null>(stand);

  useEffect(() => {
    if (stand) return;
    let aktiv = true;
    void laden().then((geladen) => {
      if (aktiv) setAusgabe(geladen);
    });
    return () => {
      aktiv = false;
    };
  }, []);

  return {
    ausgabe,
    gesperrt: (schalter) => istGesperrt(ausgabe, schalter),
  };
};
