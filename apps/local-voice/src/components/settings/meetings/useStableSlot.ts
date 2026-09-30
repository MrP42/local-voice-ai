import { useCallback, useRef } from "react";

interface SlotState {
  el: HTMLDivElement;
  lost: { target: HTMLElement; at: number } | null;
}

/** Ein Fokusverlust gilt als "durch den Umzug", wenn der Umzug so kurz danach kommt. */
const MOVE_FOCUS_WINDOW_MS = 300;

/**
 * Ein Ziel-Element, das ueber die ganze Lebensdauer der Seite dasselbe bleibt
 * und nur von einem Ort zum anderen gehaengt wird (`host`).
 *
 * Wozu: Die Aufnahmen-Seite setzt ihre Bereiche je nach Fensterbreite an
 * verschiedene Stellen (breit: drei Spalten, schmal: Aufnahmezeile oben und
 * darunter geteilt). Haengt man das Innere per React an die neue Stelle,
 * werden Notizblock (Text, Fokus, ungespeicherte Zeichen), Live-Transkript,
 * Aufnahmekarte (Uhr, Dialog) und ein laufender Chat neu gebaut. Mit einem
 * festen Element und `createPortal` bleibt der React-Baum stehen; nur das
 * DOM-Element zieht um. Das Verschieben nimmt dem Element den Fokus; er wird
 * wiederhergestellt, wenn der Verlust unmittelbar vor dem Umzug lag (ein
 * Klick ins Leere davor zaehlt nicht).
 */
export function useStableSlot(className: string, testId?: string) {
  const slot = useRef<SlotState | null>(null);
  if (slot.current === null) {
    const el = document.createElement("div");
    el.className = className;
    if (testId) el.dataset.testid = testId;
    const state: SlotState = { el, lost: null };
    el.addEventListener("focusout", (event) => {
      const target = event.target;
      state.lost =
        target instanceof HTMLElement && event.relatedTarget === null
          ? { target, at: performance.now() }
          : null;
    });
    el.addEventListener("focusin", () => {
      state.lost = null;
    });
    slot.current = state;
  }
  const { el } = slot.current;

  const host = useCallback(
    (node: HTMLElement | null) => {
      const state = slot.current;
      if (!node || !state || el.parentElement === node) return;
      node.appendChild(el);
      const lost = state.lost;
      state.lost = null;
      if (
        lost &&
        performance.now() - lost.at < MOVE_FOCUS_WINDOW_MS &&
        lost.target.isConnected &&
        document.activeElement === document.body
      ) {
        lost.target.focus({ preventScroll: true });
      }
    },
    [el],
  );

  return { el, host };
}
