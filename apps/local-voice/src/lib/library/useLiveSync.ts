import { useEffect, useRef, useState } from "react";
import { commands, type AudioSyncInsert, type SyncPoint } from "@/bindings";
import { addSpan, nextWindow, type LiveSpan } from "./liveSync";

export interface LiveSyncState {
  points: SyncPoint[];
  inserts: AudioSyncInsert[];
  covered: LiveSpan[];
  /** Erkennung nicht verfügbar (kein Modell, ffmpeg fehlt …): Schätzung bleibt. */
  unavailable: boolean;
  /** Ein Fenster wird gerade erkannt. */
  busy: boolean;
}

const EMPTY: LiveSyncState = {
  points: [],
  inserts: [],
  covered: [],
  unavailable: false,
  busy: false,
};

/** Nach einem Fehler so lange warten, bevor es erneut versucht wird. */
const RETRY_MS = 60_000;

/**
 * Live-Abgleich Hörbuch ↔ Text (Hörbuch-Sync Stufe 2): erkennt beim Hören
 * Fenster vor der Abspielstelle und liefert die Stützpunkte. Läuft nur, wenn
 * `enabled` (Mitlesen an, Buch mit Text und Hörbuch) und gespielt wird.
 */
export function useLiveSync(opts: {
  bookId: string;
  enabled: boolean;
  playing: boolean;
  positionMs: number;
  totalMs: number;
  /** Geschätzte Zeichenstelle zu einer Zeit (für den Suchhinweis). */
  hintOf(ms: number): number;
}): LiveSyncState {
  const { bookId, enabled, playing, positionMs, totalMs } = opts;
  const [state, setState] = useState<LiveSyncState>(EMPTY);
  const hintOf = useRef(opts.hintOf);
  hintOf.current = opts.hintOf;
  const busy = useRef(false);
  const failedAt = useRef(0);
  /** Hinweis aus dem letzten Fenster: gilt, wenn das nächste direkt anschließt. */
  const chain = useRef<{ toMs: number; chars: number } | null>(null);

  useEffect(() => {
    setState(EMPTY);
    chain.current = null;
    if (!enabled) return;
    let stop = false;
    void commands.audiosyncLoad(bookId).then((r) => {
      if (stop || r.status !== "ok") return;
      setState((s) => ({
        ...s,
        points: r.data.points,
        inserts: r.data.inserts,
        covered: r.data.covered,
      }));
    });
    return () => {
      stop = true;
    };
  }, [bookId, enabled]);

  // Auf 5 s gerundet: nicht bei jedem timeupdate neu prüfen.
  const tick = Math.floor(positionMs / 5000);
  useEffect(() => {
    if (!enabled || !playing || busy.current) return;
    if (Date.now() - failedAt.current < RETRY_MS) return;
    const w = nextWindow(state.covered, positionMs, totalMs);
    if (!w) return;
    const c = chain.current;
    const hint =
      c && Math.abs(c.toMs - w.from) < 1500 ? c.chars : hintOf.current(w.from);
    busy.current = true;
    setState((s) => ({ ...s, busy: true }));
    void commands
      .audiosyncWindow(bookId, w.from, w.len, Math.max(0, Math.round(hint)))
      .then((r) => {
        if (r.status !== "ok") {
          failedAt.current = Date.now();
          setState((s) => ({ ...s, unavailable: true }));
          return;
        }
        const win = r.data;
        chain.current =
          win.next_hint != null
            ? { toMs: win.to_ms, chars: win.next_hint }
            : null;
        setState((s) => {
          const inside = (ms: number) => ms >= win.from_ms && ms < win.to_ms;
          return {
            ...s,
            unavailable: false,
            points: [
              ...s.points.filter((p) => !inside(p.ms)),
              ...win.points,
            ].sort((a, b) => a.ms - b.ms),
            inserts: [
              ...s.inserts.filter((i) => !inside(i.start_ms)),
              ...win.inserts,
            ],
            covered: addSpan(s.covered, {
              from_ms: win.from_ms,
              to_ms: win.to_ms,
            }),
          };
        });
      })
      .finally(() => {
        busy.current = false;
        setState((s) => ({ ...s, busy: false }));
      });
  }, [enabled, playing, tick, state.covered, bookId, totalMs]);

  return state;
}
