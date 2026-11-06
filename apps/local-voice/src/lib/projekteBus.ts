import { useEffect, useRef } from "react";

/**
 * Kleine Meldungen zwischen den Stellen, die eine Recherche starten (Chat-Spalte, Sprachgespraech,
 * Projektseite), und den Stellen, die darauf reagieren (Freigabe-Waechter, Projektordner).
 * - `freigabe`: eine Recherche wartet auf die Freigabe ihrer Suchanfragen (oder wurde entschieden).
 * - `recherche`: eine Recherche ist fertig; `detail` ist ihre Kennung.
 * - `projekte`: Suite-Projekte, Verknuepfungen oder Eintraege haben sich geaendert.
 * - `halt-beendet`: ein Halt ist ohne Ergebnis zu Ende (`<halt>|<abgelehnt|abgelaufen|fehler>`).
 */
export type ProjekteMeldung =
  "freigabe" | "recherche" | "projekte" | "halt-beendet";

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

export type HaltEnde = "abgelehnt" | "abgelaufen" | "fehler";

export const meldeHaltEnde = (haltId: string, wie: HaltEnde) =>
  meldeProjekte("halt-beendet", `${haltId}|${wie}`);

/**
 * Wartende Laeufe nach Kennung: ein Halt (oder eine Recherche) wird erst dem Ergebnis zugeordnet, das
 * dieselbe Kennung traegt, nie "dem naechsten, das kommt". `erwarte(id)` merkt sich die Kennung;
 * `onFertig` und `onEnde` kommen nur fuer gemerkte Kennungen, danach ist sie vergessen.
 */
export function useErwartung(
  onFertig: (id: string) => void,
  onEnde: (id: string, wie: HaltEnde) => void,
) {
  const ids = useRef(new Set<string>());
  useProjekteMeldung("recherche", (id) => {
    if (id && ids.current.delete(id)) onFertig(id);
  });
  useProjekteMeldung("halt-beendet", (detail) => {
    const [id, wie] = (detail ?? "").split("|");
    if (id && ids.current.delete(id)) onEnde(id, wie as HaltEnde);
  });
  return {
    erwarte: (id: string) => void ids.current.add(id),
    vergiss: (id: string) => void ids.current.delete(id),
    leeren: () => ids.current.clear(),
  };
}
