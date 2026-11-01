import { useEffect, useState } from "react";
import { commands } from "@/bindings";

/** Alle so viele ms verschiebt sich der Inhalt um 1 px (OLED-Dimmung). */
const SHIFT_EVERY_MS = 45_000;

type WakeLockSentinelLike = { release(): Promise<void> };

/**
 * Bildschirm wach halten, solange `on` (Sonnen-Knopf im Vorlesen):
 *
 * - Betriebssystem: der Kern hält die Anzeige an (Windows
 *   `SetThreadExecutionState`, macOS `caffeinate -d`).
 * - WebView: zusätzlich die Wake-Lock-API, wo vorhanden.
 * - OLED-Monitore dimmen bei stehendem Bild trotzdem (Panel-Schutz). Dagegen
 *   wandert der Inhalt alle 45 s um 1 px — unsichtbar beim Lesen, für den
 *   Monitor aber eine Bildänderung. Der Rückgabewert ist dieser Versatz.
 *
 * Aus, beim Verlassen der Seite und beim Schließen wird alles freigegeben.
 */
export function useKeepAwake(on: boolean): number {
  const [shift, setShift] = useState(0);
  useEffect(() => {
    if (!on) {
      setShift(0);
      return;
    }
    let lock: WakeLockSentinelLike | null = null;
    let alive = true;
    const requestLock = () => {
      const wl = (
        navigator as Navigator & {
          wakeLock?: { request(kind: "screen"): Promise<WakeLockSentinelLike> };
        }
      ).wakeLock;
      wl?.request("screen")
        .then((l) => {
          if (alive) lock = l;
          else void l.release();
        })
        .catch(() => {
          /* ohne Wake-Lock hält der Kern die Anzeige an */
        });
    };
    requestLock();
    // Ein Wake-Lock fällt beim Verbergen des Fensters weg: zurück, neu holen.
    const onVisible = () => {
      if (document.visibilityState === "visible") requestLock();
    };
    document.addEventListener("visibilitychange", onVisible);
    void commands.screenKeepAwake(true).catch(() => undefined);
    const id = window.setInterval(
      () => setShift((s) => (s === 0 ? 1 : 0)),
      SHIFT_EVERY_MS,
    );
    return () => {
      alive = false;
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisible);
      void lock?.release().catch(() => undefined);
      void commands.screenKeepAwake(false).catch(() => undefined);
    };
  }, [on]);
  return shift;
}
