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

/**
 * Appweiter Zähler ab 1 (A1-Vertrag `session_generation`): jede neue Sitzung
 * bekommt einen höheren Wert, innerhalb der Sitzung bleibt er fest.
 */
let sessionCounter = 0;
const freshSession = () => {
  sessionCounter += 1;
  return {
    id: `reader-${Date.now().toString(36)}-${sessionCounter.toString(36)}`,
    generation: sessionCounter,
  };
};

/** Fehlercode ohne Präfix (`reader_live:no_local_model` → `no_local_model`). */
export const liveErrorCode = (error: string): string =>
  error.startsWith("reader_live:")
    ? error.slice("reader_live:".length)
    : "failed";

/** Leerraum gehört zur nächsten Lesestelle; UTF-16-Koordinaten bleiben erhalten. */
export function livePosition(text: string, offset: number): number {
  while (offset < text.length && /\s/u.test(text[offset])) offset++;
  return offset;
}

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
  text: string,
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
    at = livePosition(text, next) === following.start ? following : null;
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
  const [live] = useState(freshSession);
  const session = useRef(live.id);
  const epoch = useRef(0);
  /** Laufende Bestellung samt damaliger Lesestelle. */
  const inFlight = useRef<{
    epoch: number;
    from: number;
    position: number;
  } | null>(null);
  /** Abgeschlossene Bestellpositionen, auch wenn Ereignisse später eintreffen. */
  const completed = useRef(new Set<number>());
  const [listenerSource, setListenerSource] = useState<{
    pageId: string | null;
    text: string;
    target: string | null;
  } | null>(null);
  const listenerReady = useRef<typeof listenerSource>(null);
  const active = !!(target && pageId && text.trim());

  // Neue Quelle oder Sprache: alles verwerfen, laufende Arbeit abbrechen.
  useEffect(() => {
    setUnits([]);
    completed.current.clear();
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
    let disposed = false;
    listenerReady.current = null;
    setListenerSource(null);
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
    void un
      .then(() => {
        if (!disposed) {
          const source = { pageId, text, target };
          listenerReady.current = source;
          setListenerSource(source);
        }
      })
      .catch(() => {
        if (!disposed) setState({ kind: "error", code: "failed" });
      });
    return () => {
      disposed = true;
      listenerReady.current = null;
      void un.then((stop) => stop()).catch(() => {});
    };
  }, [active, pageId, text, target]);

  // Bestellen: am Anfang, nach einem Sprung und wenn der Vorlauf knapp wird.
  const position = livePosition(text, offset);
  const covered = useMemo(
    () => coveredFrom(units, position, text),
    [units, position, text],
  );
  useEffect(() => {
    if (!active || !target || !pageId) return;
    if (
      !listenerSource ||
      listenerSource !== listenerReady.current ||
      listenerSource.pageId !== pageId ||
      listenerSource.text !== text ||
      listenerSource.target !== target
    )
      return;
    if (state.kind === "existing" || state.kind === "error") return;
    const running = inFlight.current;
    // A1 meldet keinen Fensterendpunkt vorab. Nur die ursprüngliche Lesestelle
    // oder belegte Abdeckung rechtfertigen Warten; Entfernung allein nicht.
    if (running && covered.count < AHEAD) {
      if (
        position === running.position ||
        (position >= running.from && covered.count > 0)
      )
        return;
    }
    if (covered.count >= AHEAD) return;
    const from = livePosition(text, covered.count > 0 ? covered.next : position);
    if (completed.current.has(from)) return;
    if (running && running.from === from) return;
    epoch.current += 1;
    const mine = { epoch: epoch.current, from, position };
    inFlight.current = mine;
    setState({ kind: "translating" });
    void commands
      .readerLiveTranslate({
        session_id: session.current,
        session_generation: live.generation,
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
        completed.current.add(from);
        setState({ kind: "ready" });
      })
      .catch(() => {
        if (inFlight.current !== mine) return;
        inFlight.current = null;
        setState({ kind: "error", code: "failed" });
      });
  }, [
    active,
    target,
    pageId,
    text,
    rate,
    position,
    covered,
    state.kind,
    listenerSource,
  ]);

  return { state, units };
}
