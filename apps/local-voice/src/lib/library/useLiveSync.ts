import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type AudioSyncInsert,
  type AudioSyncWindow,
  type SyncPoint,
} from "@/bindings";
import { addSpan, nextWindow, type LiveSpan } from "./liveSync";

export interface LiveSyncState {
  points: SyncPoint[];
  inserts: AudioSyncInsert[];
  covered: LiveSpan[];
  /** Von Hand gesetzte Stellen (Vorrang vor allem Erkannten). */
  anchors: SyncPoint[];
  /** Gespeicherter Abgleich geladen (oder gar keiner nötig). */
  loaded: boolean;
  /** Erkennung nicht verfügbar (kein Modell, ffmpeg fehlt …): Schätzung bleibt. */
  unavailable: boolean;
  /** Ein Fenster wird gerade erkannt. */
  busy: boolean;
}

const EMPTY: LiveSyncState = {
  points: [],
  inserts: [],
  covered: [],
  anchors: [],
  loaded: false,
  unavailable: false,
  busy: false,
};

/** Nach einem Fehler so lange warten, bevor es erneut versucht wird. */
const RETRY_MS = 60_000;
/** Fensterlänge bei der Suche nach der Startstelle (kurz = schnell). */
const SEARCH_WINDOW_MS = 15_000;
/** So viele Fenster höchstens, bis die Startstelle gefunden sein muss. */
const SEARCH_STEPS = 3;

export interface LiveSync extends LiveSyncState {
  /** Hand-Stelle setzen: bei `ms` ist Zeichen `chars` zu hören. */
  anchor(chars: number, ms: number): Promise<void>;
  /**
   * Zeit zu einer Zeichenstelle durch Erkennen suchen (Start an der zuletzt
   * gelesenen Stelle). `estimateMs`: Schätzung; `msPerChar`: Tempo für den
   * nächsten Versuch. null, wenn nicht gefunden oder nicht möglich.
   */
  find(
    chars: number,
    estimateMs: number,
    msPerChar: number,
  ): Promise<number | null>;
}

/** Ergebnis eines Fensters in den Zustand übernehmen. */
function withWindow(s: LiveSyncState, win: AudioSyncWindow): LiveSyncState {
  const inside = (ms: number) => ms >= win.from_ms && ms < win.to_ms;
  return {
    ...s,
    unavailable: false,
    points: [...s.points.filter((p) => !inside(p.ms)), ...win.points].sort(
      (a, b) => a.ms - b.ms,
    ),
    inserts: [
      ...s.inserts.filter((i) => !inside(i.start_ms)),
      ...win.inserts,
    ].sort((a, b) => a.start_ms - b.start_ms),
    covered: addSpan(s.covered, { from_ms: win.from_ms, to_ms: win.to_ms }),
  };
}

/** Hand-Stelle einfügen wie im Kern: ersetzt nahe und widersprechende. */
function withAnchor(anchors: SyncPoint[], p: SyncPoint): SyncPoint[] {
  return [
    ...anchors.filter(
      (a) =>
        Math.abs(a.ms - p.ms) > 5000 &&
        ((a.ms < p.ms && a.chars < p.chars) ||
          (a.ms > p.ms && a.chars > p.chars)),
    ),
    p,
  ].sort((a, b) => a.ms - b.ms);
}

/**
 * Live-Abgleich Hörbuch ↔ Text (Hörbuch-Sync Stufe 2/3): erkennt beim Hören
 * Fenster vor der Abspielstelle und liefert die Stützpunkte. Läuft nur, wenn
 * `enabled` (Buch mit Text und Hörbuch, ohne Transkript) und gespielt wird.
 */
export function useLiveSync(opts: {
  bookId: string;
  enabled: boolean;
  playing: boolean;
  positionMs: number;
  totalMs: number;
  /** Geschätzte Zeichenstelle zu einer Zeit (für den Suchhinweis). */
  hintOf(ms: number): number;
}): LiveSync {
  const { bookId, enabled, playing, positionMs, totalMs } = opts;
  const [state, setState] = useState<LiveSyncState>(EMPTY);
  const hintOf = useRef(opts.hintOf);
  hintOf.current = opts.hintOf;
  const busy = useRef(false);
  const failedAt = useRef(0);
  /** Hinweis aus dem letzten Fenster: gilt, wenn das nächste direkt anschließt. */
  const chain = useRef<{ toMs: number; chars: number } | null>(null);
  /** Zählt Buchwechsel: Antworten zum alten Buch werden verworfen. */
  const gen = useRef(0);

  useEffect(() => {
    gen.current += 1;
    const mine = gen.current;
    setState({ ...EMPTY, loaded: !enabled });
    chain.current = null;
    busy.current = false;
    failedAt.current = 0;
    if (!enabled) return;
    void commands.audiosyncLoad(bookId).then((r) => {
      if (mine !== gen.current) return;
      setState((s) =>
        r.status === "ok"
          ? {
              ...s,
              loaded: true,
              points: r.data.points,
              inserts: r.data.inserts,
              covered: r.data.covered,
              anchors: r.data.anchors ?? [],
            }
          : { ...s, loaded: true },
      );
    });
  }, [bookId, enabled]);

  /** Ein Fenster erkennen; null bei Fehler (dann Pause bis RETRY_MS). */
  const runWindow = useCallback(
    async (from: number, len: number, hint: number) => {
      const mine = gen.current;
      busy.current = true;
      setState((s) => ({ ...s, busy: true }));
      try {
        const r = await commands.audiosyncWindow(
          bookId,
          from,
          len,
          Math.max(0, Math.round(hint)),
        );
        if (mine !== gen.current) return null;
        if (r.status !== "ok") {
          failedAt.current = Date.now();
          setState((s) => ({ ...s, unavailable: true }));
          return null;
        }
        const win = r.data;
        if (!(win.to_ms > win.from_ms)) {
          failedAt.current = Date.now();
          return null;
        }
        chain.current =
          win.next_hint != null
            ? { toMs: win.to_ms, chars: win.next_hint }
            : null;
        setState((s) => withWindow(s, win));
        return win;
      } catch {
        if (mine === gen.current) failedAt.current = Date.now();
        return null;
      } finally {
        if (mine === gen.current) {
          busy.current = false;
          setState((s) => ({ ...s, busy: false }));
        }
      }
    },
    [bookId],
  );

  // Auf 5 s gerundet: nicht bei jedem timeupdate neu prüfen.
  const tick = Math.floor(positionMs / 5000);
  useEffect(() => {
    if (!enabled || !state.loaded || !playing || busy.current) return;
    if (Date.now() - failedAt.current < RETRY_MS) return;
    const w = nextWindow(state.covered, positionMs, totalMs);
    if (!w) return;
    const c = chain.current;
    const hint =
      c && Math.abs(c.toMs - w.from) < 1500 ? c.chars : hintOf.current(w.from);
    void runWindow(w.from, w.len, hint);
  }, [enabled, state.loaded, playing, tick, state.covered, totalMs, runWindow]);

  const anchor = useCallback(
    async (chars: number, ms: number) => {
      const mine = gen.current;
      const p = { chars: Math.max(0, Math.round(chars)), ms: Math.max(0, ms) };
      // Sofort wirksam; die gespeicherte Liste ersetzt sie danach.
      setState((s) => ({ ...s, anchors: withAnchor(s.anchors, p) }));
      chain.current = null;
      const r = await commands.audiosyncAnchor(bookId, p.chars, p.ms);
      if (mine !== gen.current || r.status !== "ok") return;
      setState((s) => ({ ...s, anchors: r.data.anchors ?? s.anchors }));
    },
    [bookId],
  );

  const find = useCallback(
    async (chars: number, estimateMs: number, msPerChar: number) => {
      if (!enabled || busy.current) return null;
      let est = Math.max(0, estimateMs);
      for (let step = 0; step < SEARCH_STEPS; step++) {
        const from = Math.max(0, est - 3000);
        const win = await runWindow(from, SEARCH_WINDOW_MS, chars);
        const pts = win?.points ?? [];
        if (pts.length < 2) return null;
        const first = pts[0];
        const last = pts[pts.length - 1];
        if (chars >= first.chars && chars <= last.chars) {
          for (let i = 1; i < pts.length; i++) {
            const a = pts[i - 1];
            const b = pts[i];
            if (chars <= b.chars) {
              const f =
                b.chars > a.chars ? (chars - a.chars) / (b.chars - a.chars) : 0;
              return a.ms + f * (b.ms - a.ms);
            }
          }
          return last.ms;
        }
        const near = chars < first.chars ? first : last;
        est = Math.max(0, near.ms + (chars - near.chars) * msPerChar);
        if (totalMs > 0) est = Math.min(est, Math.max(0, totalMs - 1000));
      }
      return null;
    },
    [enabled, runWindow, totalMs],
  );

  return { ...state, anchor, find };
}
