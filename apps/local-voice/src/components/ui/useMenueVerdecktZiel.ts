import { useCallback, useSyncExternalStore } from "react";
import { istMenueOffen } from "./menueOffen";

/** Ein Beobachter auf `body` fuer alle Tooltips (statt einer je Tooltip). */
const listeners = new Set<() => void>();
let observer: MutationObserver | null = null;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!observer && typeof MutationObserver !== "undefined") {
    // Mehrere Aenderungen eines Bildes werden zu einer Meldung gebuendelt.
    let geplant = false;
    observer = new MutationObserver(() => {
      if (geplant) return;
      geplant = true;
      requestAnimationFrame(() => {
        geplant = false;
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
      istMenueOffen(document, targetRef.current),
    [targetRef],
  );
  return useSyncExternalStore(subscribe, snapshot, () => false);
}
