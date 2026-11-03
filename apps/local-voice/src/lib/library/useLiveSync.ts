import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type AudioSyncInsert,
  type AudioSyncWindow,
  type SyncPoint,
} from "@/bindings";
import { addSpan, monotoneChain, nextWindow, type LiveSpan } from "./liveSync";

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

interface InnerState extends Omit<LiveSyncState, "loaded"> {
  /** Buch, für das der gespeicherte Abgleich geladen ist. */
  loadedFor: string | null;
}

const EMPTY: InnerState = {
  points: [],
  inserts: [],
  covered: [],
  anchors: [],
  loadedFor: null,
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
    opts: { msPerChar: number; totalMs: number; abort: { aborted: boolean } },
  ): Promise<number | null>;
}

/** Ergebnis eines Fensters in den Zustand übernehmen. */
function withWindow(s: InnerState, win: AudioSyncWindow): InnerState {
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
  const [state, setState] = useState<InnerState>(EMPTY);
  // Pro Buch: ein veraltetes „geladen“ vom vorigen Buch zählt nicht.
  const loaded = !enabled || state.loadedFor === bookId;
  const hintOf = useRef(opts.hintOf);
  hintOf.current = opts.hintOf;
  /** Laufende oder wartende Fenster (die Erkennung läuft seriell). */
  const pending = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  /** Startsuche läuft: die Fenster beim Hören warten. */
  const searching = useRef(false);
  const failedAt = useRef(0);
  /** Hinweis aus dem letzten Fenster: gilt, wenn das nächste direkt anschließt. */
  const chain = useRef<{ toMs: number; chars: number } | null>(null);
  /** Zählt Buchwechsel: Antworten zum alten Buch werden verworfen. */
  const gen = useRef(0);

  useEffect(() => {
    gen.current += 1;
    const mine = gen.current;
    setState(EMPTY);
    chain.current = null;
    failedAt.current = 0;
    if (!enabled) return;
    void commands.audiosyncLoad(bookId).then((r) => {
      if (mine !== gen.current) return;
      setState((s) =>
        r.status === "ok"
          ? {
              ...s,
              loadedFor: bookId,
              points: r.data.points,
              inserts: r.data.inserts,
              covered: r.data.covered,
              anchors: r.data.anchors ?? [],
            }
          : { ...s, loadedFor: bookId },
      );
    });
  }, [bookId, enabled]);

  /** Ein Fenster erkennen; null bei Fehler (dann Pause bis RETRY_MS). */
  const runOne = useCallback(
    async (mine: number, from: number, len: number, hint: number) => {
      // In der Warteschlange das Buch gewechselt: nichts mehr erkennen.
      if (mine !== gen.current) return null;
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
        if (mine === gen.current) setState((s) => ({ ...s, busy: false }));
      }
    },
    [bookId],
  );
  /** Fenster nacheinander, nie zwei Erkennungen zugleich. */
  const runWindow = useCallback(
    (from: number, len: number, hint: number) => {
      const mine = gen.current;
      pending.current += 1;
      const p = queue.current
        .then(() => runOne(mine, from, len, hint))
        .finally(() => {
          pending.current -= 1;
        });
      queue.current = p.catch(() => null);
      return p;
    },
    [runOne],
  );

  // Auf 5 s gerundet: nicht bei jedem timeupdate neu prüfen.
  const tick = Math.floor(positionMs / 5000);
  useEffect(() => {
    if (!enabled || !loaded || !playing) return;
    if (pending.current > 0 || searching.current) return;
    if (Date.now() - failedAt.current < RETRY_MS) return;
    const w = nextWindow(state.covered, positionMs, totalMs);
    if (!w) return;
    const c = chain.current;
    const hint =
      c && Math.abs(c.toMs - w.from) < 1500 ? c.chars : hintOf.current(w.from);
    void runWindow(w.from, w.len, hint);
  }, [enabled, loaded, playing, tick, state.covered, totalMs, runWindow]);

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
    async (
      chars: number,
      estimateMs: number,
      o: { msPerChar: number; totalMs: number; abort: { aborted: boolean } },
    ) => {
      if (!enabled) return null;
      searching.current = true;
      try {
        let est = Math.max(0, estimateMs);
        for (let step = 0; step < SEARCH_STEPS; step++) {
          if (o.abort.aborted) return null;
          const from = Math.max(0, est - 3000);
          const win = await runWindow(from, SEARCH_WINDOW_MS, chars);
          if (!win || o.abort.aborted) return null;
          // Ausreißer zählen nicht: nur die monotone Kette des Fensters.
          const pts = monotoneChain(win.points);
          if (!pts.length) return null;
          const first = pts[0];
          const last = pts[pts.length - 1];
          if (pts.length > 1 && chars >= first.chars && chars <= last.chars) {
            for (let i = 1; i < pts.length; i++) {
              const a = pts[i - 1];
              const b = pts[i];
              if (chars <= b.chars) {
                const f =
                  b.chars > a.chars
                    ? (chars - a.chars) / (b.chars - a.chars)
                    : 0;
                return a.ms + f * (b.ms - a.ms);
              }
            }
            return last.ms;
          }
          const near = chars < first.chars ? first : last;
          est = Math.max(0, near.ms + (chars - near.chars) * o.msPerChar);
          if (o.totalMs > 0) est = Math.min(est, Math.max(0, o.totalMs - 1000));
        }
        return null;
      } finally {
        searching.current = false;
      }
    },
    [enabled, runWindow],
  );

  const { loadedFor: _loadedFor, ...rest } = state;
  return { ...rest, loaded, anchor, find };
}
