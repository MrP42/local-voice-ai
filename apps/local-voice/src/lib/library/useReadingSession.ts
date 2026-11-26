import { useEffect, useRef } from "react";
import { beginReadingSession, flushProgress, type ReadingLogEntry } from "./progress";

/** Play/Pause, Seitenwechsel und Verlassen teilen sich denselben Schreibweg. */
export function useReadingSession(
  bookId: string | null,
  edition: string,
  medium: ReadingLogEntry["medium"],
  unit: ReadingLogEntry["unit"],
  active: boolean,
  position: number,
  playing = false,
): void {
  const at = useRef(position);
  const saveRef = useRef<() => void>(() => {});
  const identity = `${bookId}:${edition}:${medium}:${unit}`;
  const previousIdentity = useRef(identity);
  // Beim Buch-/Fassungswechsel sichert der alte Effekt noch seine alte Stelle.
  if (previousIdentity.current === identity) at.current = position;
  useEffect(() => {
    previousIdentity.current = identity;
    at.current = position;
    if (!bookId || !active) return;
    let checkpoint: ((to: number) => void) | null = null;
    const begin = () => {
      checkpoint ??= beginReadingSession(bookId, { edition, medium, unit, from: at.current });
    };
    const save = () => checkpoint?.(at.current);
    saveRef.current = save;
    const end = () => { save(); checkpoint = null; void flushProgress(); };
    const visibility = () => {
      if (document.visibilityState === "hidden") {
        if (!playing) end();
        else { save(); void flushProgress(); }
      } else begin();
    };
    if (playing || document.visibilityState !== "hidden") begin();
    const timer = window.setInterval(save, 5000);
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", end);
    return () => {
      end();
      saveRef.current = () => {};
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", end);
    };
  }, [bookId, edition, medium, unit, active, identity, playing]);
  useEffect(() => { saveRef.current(); }, [position]);
}
