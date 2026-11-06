import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

export interface FileDropState {
  /** Eine Datei schwebt über dem Bereich. */
  over: boolean;
  /** Buch unter dem Zeiger (`data-drop-book`), sonst `null` = freie Fläche. */
  targetId: string | null;
}

const IDLE: FileDropState = { over: false, targetId: null };

/** Element unter einem Punkt in physischen Fensterpixeln (Tauri-Ereignis). */
function hit(position: { x: number; y: number }): Element | null {
  const scale = window.devicePixelRatio || 1;
  return document.elementFromPoint(position.x / scale, position.y / scale);
}

/**
 * Dateien aus dem Explorer/Finder ablegen (lva#152). Tauri fängt Datei-Drops
 * fensterweit ab (HTML5-Ziehen liefert keine Pfade); darum das Tauri-Ereignis
 * mit Trefferprüfung: nur über `rootSelector` zählt der Drop, ein Element mit
 * `data-drop-book` darunter ist das Ziel.
 */
export function useFileDrop(
  rootSelector: string,
  onDrop: (paths: string[], targetId: string | null) => void,
): FileDropState {
  const [state, setState] = useState<FileDropState>(IDLE);
  const dropRef = useRef(onDrop);
  dropRef.current = onDrop;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const locate = (position: { x: number; y: number }) => {
      const el = hit(position);
      if (!el?.closest(rootSelector)) return null;
      return {
        targetId: el.closest("[data-drop-book]")?.getAttribute("data-drop-book") ?? null,
      };
    };
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "leave") {
          setState(IDLE);
          return;
        }
        const at = locate(payload.position);
        if (payload.type !== "drop") {
          setState((prev) => {
            const next = at ? { over: true, targetId: at.targetId } : IDLE;
            return prev.over === next.over && prev.targetId === next.targetId
              ? prev
              : next;
          });
          return;
        }
        setState(IDLE);
        if (at && payload.paths.length) dropRef.current(payload.paths, at.targetId);
      })
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [rootSelector]);

  return state;
}
