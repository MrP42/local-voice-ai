import { useEffect, useRef } from "react";

/**
 * Kleine Meldungen zwischen den Stellen, die eine Recherche starten (Chat-Spalte, Sprachgespraech,
 * Projektseite), und den Stellen, die darauf reagieren (Freigabe-Waechter, Projektordner).
 * - `freigabe`: eine Recherche wartet auf die Freigabe ihrer Suchanfragen (oder wurde entschieden).
 * - `recherche`: eine Recherche ist fertig; `detail` ist ihre Kennung.
 * - `projekte`: Suite-Projekte, Verknuepfungen oder Eintraege haben sich geaendert.
 */
export type ProjekteMeldung = "freigabe" | "recherche" | "projekte";

const EVENT = "lva:projekte";

export const meldeProjekte = (art: ProjekteMeldung, detail?: string) => {
  window.dispatchEvent(new CustomEvent(EVENT, { detail: { art, detail } }));
};

export const useProjekteMeldung = (
  art: ProjekteMeldung,
  callback: (detail?: string) => void,
) => {
  const ref = useRef(callback);
  ref.current = callback;
  useEffect(() => {
    const handler = (e: Event) => {
      const d = (e as CustomEvent<{ art: ProjekteMeldung; detail?: string }>)
        .detail;
      if (d?.art === art) ref.current(d.detail);
    };
    window.addEventListener(EVENT, handler);
    return () => window.removeEventListener(EVENT, handler);
  }, [art]);
};
