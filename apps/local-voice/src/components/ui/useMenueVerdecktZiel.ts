import { useCallback, useSyncExternalStore } from "react";
import { istMenueOffen } from "./menueOffen";

/** Ein Beobachter auf `body` fuer alle Tooltips (statt einer je Tooltip). */
const listeners = new Set<() => void>();
let observer: MutationObserver | null = null;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!observer && typeof MutationObserver !== "undefined") {
    observer = new MutationObserver(() => listeners.forEach((l) => l()));
    observer.observe(document.body, { childList: true, subtree: true });
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
