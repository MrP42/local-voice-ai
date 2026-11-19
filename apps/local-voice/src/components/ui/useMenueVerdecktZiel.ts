import { useCallback, useSyncExternalStore } from "react";
import {
  MENUE_SELECTOR,
  menueVerdecktZiel,
  sichtbareMenues,
} from "./menueOffen";

/**
 * Ein Beobachter auf `body` fuer alle Tooltips (statt einer je Tooltip). Die Liste der
 * sichtbaren Menues wird einmal je Aenderungsschub berechnet und geteilt (lva#692); der
 * Snapshot jedes Tooltips prueft nur noch gegen diese Liste.
 */
const listeners = new Set<() => void>();
let observer: MutationObserver | null = null;
let menues: Element[] | null = null;

const kannMenueSein = (knoten: Node): boolean =>
  knoten instanceof Element &&
  (knoten.matches(MENUE_SELECTOR) ||
    knoten.querySelector(MENUE_SELECTOR) !== null);

/** Betrifft der Aenderungsschub ein Menue (hinzugefuegt, entfernt, Attribut am Menue)? */
const betrifftMenue = (records: MutationRecord[]): boolean =>
  records.some((r) =>
    r.type === "attributes"
      ? r.attributeName === "role" || kannMenueSein(r.target)
      : [...r.addedNodes, ...r.removedNodes].some(kannMenueSein),
  );

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!observer && typeof MutationObserver !== "undefined") {
    menues = sichtbareMenues(document);
    // Mehrere Aenderungen eines Bildes werden zu einer Meldung gebuendelt.
    let geplant = false;
    observer = new MutationObserver((records) => {
      if (!betrifftMenue(records) || geplant) return;
      geplant = true;
      requestAnimationFrame(() => {
        geplant = false;
        menues = sichtbareMenues(document);
        listeners.forEach((l) => l());
      });
    });
    // Menues koennen auch ueber Attribute erscheinen oder verschwinden.
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["role", "hidden", "aria-hidden", "style", "class"],
    });
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && observer) {
      observer.disconnect();
      observer = null;
      menues = null;
    }
  };
}

/** `true`, solange ein Menue offen ist, das das Ziel nicht selbst enthaelt. */
export function useMenueVerdecktZiel(
  targetRef: React.RefObject<HTMLElement | null>,
): boolean {
  const snapshot = useCallback(
    () =>
      typeof document !== "undefined" &&
      menueVerdecktZiel(menues ?? sichtbareMenues(document), targetRef.current),
    [targetRef],
  );
  return useSyncExternalStore(subscribe, snapshot, () => false);
}
