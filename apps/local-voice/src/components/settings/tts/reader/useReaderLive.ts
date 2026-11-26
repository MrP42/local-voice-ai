import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type ReaderLiveSentence } from "@/bindings";

/** Übersetzter Originalsatz, Bereich in UTF-16 im unveränderten Original. */
export interface LiveUnit {
  start: number;
  end: number;
  translation: string;
}

export type LiveState =
  | { kind: "off" }
  | { kind: "translating" }
  | { kind: "ready" }
  | { kind: "existing"; translation: string }
  | { kind: "error"; code: string };

/** Wie viele fertige Sätze vor der Lesestelle liegen sollen, bevor nachbestellt wird. */
const AHEAD = 3;

let sessionCounter = 0;
const freshSession = () =>
  `reader-${Date.now().toString(36)}-${(sessionCounter++).toString(36)}`;

/** Fehlercode ohne Präfix (`reader_live:no_local_model` → `no_local_model`). */
export const liveErrorCode = (error: string): string =>
  error.startsWith("reader_live:")
    ? error.slice("reader_live:".length)
    : "failed";

/** Einheit, die `offset` enthält (sonst `null`). Einheiten nach `start` sortiert. */
export function unitAt(units: LiveUnit[], offset: number): LiveUnit | null {
  let lo = 0;
  let hi = units.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const u = units[mid];
    if (offset < u.start) hi = mid - 1;
    else if (offset >= u.end) lo = mid + 1;
    else return u;
  }
  return null;
}

/** Fertige Einheiten lückenlos ab `offset`: wie viele, und wo die Lücke beginnt. */
export function coveredFrom(
  units: LiveUnit[],
  offset: number,
): { count: number; next: number } {
  let at = unitAt(units, offset);
  if (!at) return { count: 0, next: offset };
  let count = 0;
  let next = at.start;
  while (at) {
    count++;
    next = at.end;
    // Leerraum zwischen zwei Sätzen überspringen: der nächste Satz beginnt
    // beim ersten Zeichen nach `next`, das zu einer Einheit gehört.
    const following = units.find((u) => u.start >= next);
    if (!following) break;
    at = following.start - next <= 4 ? following : null;
  }
  return { count, next };
}

/**
 * Live-Übersetzung des Originals beim Lesen (lva#782 A2a).
 *
 * Bestellt ab der Lesestelle ein Vorlauffenster beim lokalen Modell
 * (`reader_live_translate`), sammelt die Sätze aus `reader-live-sentence` und
 * bestellt nach, sobald weniger als {@link AHEAD} fertige Sätze vor der
 * Lesestelle liegen. Ein Sprung außerhalb des Fertigen bestellt mit neuer
 * Epoche ab der neuen Stelle. Nur Arbeitsspeicher; ohne Ziel ist alles aus.
 */
export function useReaderLive({
  pageId,
  text,
  target,
  offset,
  rate,
}: {
  pageId: string | null;
  text: string;
  /** Zielsprache als Kennung (`en`), `null`: aus. */
  target: string | null;
  /** Aktuelle Lesestelle im Original (UTF-16). */
  offset: number;
  rate: number;
}): { state: LiveState; units: LiveUnit[] } {
  const [units, setUnits] = useState<LiveUnit[]>([]);
  const [state, setState] = useState<LiveState>({ kind: "off" });
  const session = useRef(freshSession());
  const epoch = useRef(0);
  /** Laufende Bestellung: ihre Epoche und ab wo. */
  const inFlight = useRef<{ epoch: number; from: number } | null>(null);
  /** Nach dem Textende gibt es nichts mehr zu bestellen. */
  const atEnd = useRef(false);
  const active = !!(target && pageId && text.trim());

  // Neue Quelle oder Sprache: alles verwerfen, laufende Arbeit abbrechen.
  useEffect(() => {
    setUnits([]);
    atEnd.current = false;
    inFlight.current = null;
    setState(active ? { kind: "translating" } : { kind: "off" });
    return () => {
      // Abbruch heißt: eine neuere Epoche ansagen.
      epoch.current += 1;
      inFlight.current = null;
      void commands
        .readerLiveCancel(session.current, epoch.current)
        .catch(() => false);
    };
  }, [active, pageId, text, target]);

  // Sätze der aktuellen Sitzung und Epoche einsammeln.
  useEffect(() => {
    if (!active) return;
    const un = listen<ReaderLiveSentence>("reader-live-sentence", (e) => {
      const p = e.payload;
      // Späte Ereignisse alter Sitzungen oder Epochen können noch in der
      // Warteschlange der Webview liegen.
      if (p.session_id !== session.current || p.page_id !== pageId) return;
      // Nur die jüngste Bestellung zählt; ihre letzten Sätze dürfen auch nach
      // dem Ende des Aufrufs eintreffen.
      if (p.epoch !== epoch.current || p.target_lang !== target) return;
      setUnits((prev) => {
        if (prev.some((u) => u.start === p.source_start)) return prev;
        const next = [
          ...prev,
          {
            start: p.source_start,
            end: p.source_end,
            translation: p.translation,
          },
        ];
        next.sort((a, b) => a.start - b.start);
        return next;
      });
    });
    return () => {
      void un.then((f) => f());
    };
  }, [active, pageId, target]);

  // Bestellen: am Anfang, nach einem Sprung und wenn der Vorlauf knapp wird.
  const covered = useMemo(() => coveredFrom(units, offset), [units, offset]);
  useEffect(() => {
    if (!active || !target || !pageId) return;
    if (state.kind === "existing" || state.kind === "error") return;
    const running = inFlight.current;
    // Läuft schon eine Bestellung, die diese Stelle abdeckt: warten.
    if (running && offset >= running.from && covered.count < AHEAD) {
      if (covered.count > 0 || offset - running.from < 4000) return;
    }
    if (covered.count >= AHEAD) return;
    if (atEnd.current && covered.count > 0) return;
    const from = covered.count > 0 ? covered.next : offset;
    if (running && running.from === from) return;
    epoch.current += 1;
    const mine = { epoch: epoch.current, from };
    inFlight.current = mine;
    setState({ kind: "translating" });
    void commands
      .readerLiveTranslate({
        session_id: session.current,
        page_id: pageId,
        text,
        target_lang: target,
        source_offset: from,
        epoch: mine.epoch,
        rate,
      })
      .then((result) => {
        if (inFlight.current !== mine) return;
        inFlight.current = null;
        if (result.status === "error") {
          const code = liveErrorCode(result.error);
          // Abgelöst durch eine neuere Bestellung: kein Fehler.
          if (code === "stale_epoch" || code === "cancelled") return;
          setState({ kind: "error", code });
          return;
        }
        if (!result.data) {
          setState({ kind: "error", code: "failed" });
          return;
        }
        if (result.data.kind === "existing") {
          setState({ kind: "existing", translation: result.data.translation });
          return;
        }
        if (result.data.sentences === 0) atEnd.current = true;
        setState({ kind: "ready" });
      })
      .catch(() => {
        if (inFlight.current !== mine) return;
        inFlight.current = null;
        setState({ kind: "error", code: "failed" });
      });
  }, [active, target, pageId, text, rate, offset, covered, state.kind]);

  return { state, units };
}
