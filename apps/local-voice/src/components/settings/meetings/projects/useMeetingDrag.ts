import { useCallback, useEffect, useRef, useState } from "react";
import type React from "react";
import type { Meeting } from "@/bindings";

/** Ab dieser Strecke (Pixel) wird aus einem Klick ein Ziehen. */
const DRAG_THRESHOLD = 6;
/** Attribut an jedem Ziel: der Wert ist die Projekt-ID, "all" oder "none". */
export const DROP_ATTR = "data-project-drop";

export interface MeetingDrag {
  meeting: Meeting;
  x: number;
  y: number;
  /** Strg (macOS: Cmd) gedrueckt: zum Projekt hinzufuegen statt verschieben. */
  additive: boolean;
  /** Das Ziel unter dem Zeiger, sonst `null`. */
  target: string | null;
}

const targetAt = (x: number, y: number): string | null =>
  document
    .elementFromPoint(x, y)
    ?.closest<HTMLElement>(`[${DROP_ATTR}]`)
    ?.getAttribute(DROP_ATTR) ?? null;

/**
 * Besprechungen mit dem Zeiger auf ein Projekt ziehen.
 *
 * Bewusst mit Zeigerereignissen statt HTML5-Ziehen: Tauri faengt unter Windows
 * Ziehvorgaenge im Fenster selbst ab (Dateiablage fuer den Import), dann kommen
 * `dragstart`/`drop` der Seite nie an. Zeigerereignisse laufen ueberall gleich.
 *
 * `start` gehoert an `onPointerDown` einer Zeile. Nach dem Loslassen
 * unterdrueckt `consumeClick()` den Klick, der sonst die Zeile oeffnen wuerde.
 * Escape bricht ab.
 */
export function useMeetingDrag(
  onDrop: (meeting: Meeting, target: string, additive: boolean) => void,
) {
  const [drag, setDrag] = useState<MeetingDrag | null>(null);
  const suppressClick = useRef(false);
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;
  const cleanup = useRef<(() => void) | null>(null);

  // Beim Ausblenden der Seite darf kein Zuhoerer haengen bleiben.
  useEffect(() => () => cleanup.current?.(), []);

  const start = useCallback((event: React.PointerEvent, meeting: Meeting) => {
    if (event.button !== 0 || event.pointerType === "touch") return;
    const x0 = event.clientX;
    const y0 = event.clientY;
    let active = false;
    let additive = event.ctrlKey || event.metaKey;

    const update = (x: number, y: number) =>
      setDrag({ meeting, x, y, additive, target: targetAt(x, y) });

    const onMove = (e: PointerEvent) => {
      if (!active) {
        if (Math.hypot(e.clientX - x0, e.clientY - y0) < DRAG_THRESHOLD) return;
        active = true;
        document.body.style.userSelect = "none";
      }
      additive = e.ctrlKey || e.metaKey;
      update(e.clientX, e.clientY);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        active = false;
        finish();
        return;
      }
      if (e.key === "Control" || e.key === "Meta") {
        additive = e.type === "keydown";
        setDrag((d) => (d ? { ...d, additive } : d));
      }
    };
    const finish = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
      document.body.style.userSelect = "";
      cleanup.current = null;
      setDrag(null);
    };
    const onUp = (e: PointerEvent) => {
      const wasActive = active;
      const target = wasActive ? targetAt(e.clientX, e.clientY) : null;
      const ctrl = e.ctrlKey || e.metaKey || additive;
      finish();
      if (!wasActive) return;
      suppressClick.current = true;
      window.setTimeout(() => {
        suppressClick.current = false;
      }, 0);
      if (target) onDropRef.current(meeting, target, ctrl);
    };
    const onCancel = () => {
      active = false;
      finish();
    };

    cleanup.current?.();
    cleanup.current = finish;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
  }, []);

  const consumeClick = useCallback(() => {
    const was = suppressClick.current;
    suppressClick.current = false;
    return was;
  }, []);

  return { drag, start, consumeClick };
}
