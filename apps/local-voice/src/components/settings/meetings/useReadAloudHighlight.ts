import { useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import {
  buildCharStream,
  canPaintHighlights,
  locateSentence,
  normalizeForMatch,
  rangesOfSpan,
  type CharStream,
  type MatchSpan,
} from "@/lib/readAloudHighlight";
import { getReadAloudSession, subscribeReadAloud } from "./readAloudSession";

/** Name des Highlights; die Farben stehen in App.css (`::highlight(readaloud-cur)`). */
export const READ_ALOUD_HIGHLIGHT = "readaloud-cur";

/** Das nächste scrollbare Elternelement (sonst das Dokument). */
function scrollParent(el: Element | null): Element | null {
  for (let p = el?.parentElement ?? null; p; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    if (
      (overflowY === "auto" || overflowY === "scroll") &&
      p.scrollHeight > p.clientHeight
    )
      return p;
  }
  return document.scrollingElement;
}

/** Den Satz ins Bild holen, wenn er außerhalb steht; sonst nichts bewegen. */
function revealRange(at: StaticRange) {
  const range = document.createRange();
  range.setStart(at.startContainer, at.startOffset);
  range.setEnd(at.endContainer, at.endOffset);
  const rect = range.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return;
  const container = scrollParent(range.startContainer.parentElement ?? null);
  if (!container) return;
  const isRoot = container === document.scrollingElement;
  const box = isRoot
    ? { top: 0, bottom: window.innerHeight }
    : container.getBoundingClientRect();
  // Klebende Leisten oben (Werkzeugzeile mit dem Player) verdecken die ersten Zeilen:
  // sichtbar ist erst, was unter ihnen steht.
  let top = box.top;
  for (const el of container.querySelectorAll(".sticky")) {
    const r = el.getBoundingClientRect();
    if (r.height > 0 && r.top <= top + 4 && r.bottom > top) top = r.bottom;
  }
  if (rect.top >= top + 4 && rect.bottom <= box.bottom - 8) return;
  const delta = rect.top - top - (box.bottom - top) / 3;
  container.scrollBy({ top: delta, behavior: "smooth" });
}

/**
 * Hebt im Dokument unter `rootRef` den Satz hervor, den die Vorlese-Sitzung `speakKey`
 * gerade spricht (`tts-current-sentence`), und holt ihn ins Bild. Gleiche Darstellung wie
 * im Vorlesen-Modul, gemalt über die CSS Custom Highlight API (keine Spans).
 *
 * `content` ist der angezeigte Text (z. B. das Markdown): ändert er sich, gilt die alte
 * Zuordnung nicht mehr. Steht unter `rootRef` gerade kein Dokument (Bearbeitungsmodus,
 * Ladezustand), bleibt die Hervorhebung weg.
 */
export function useReadAloudHighlight(
  rootRef: RefObject<HTMLElement | null>,
  speakKey: string,
  content: unknown,
) {
  const session = useSyncExternalStore(subscribeReadAloud, getReadAloudSession);
  const mine = session?.key === speakKey ? session : null;
  const active = mine !== null;
  const token = mine?.token ?? null;
  const index = mine?.sentence?.index ?? null;
  const text = mine?.sentence?.text ?? null;

  // Rendert die Ansicht das Dokument neu (gleicher Text, neue Textknoten), zeigen alte
  // Ranges ins Leere: ein MutationObserver am Wurzelelement zählt jede Änderung mit.
  const [domVersion, setDomVersion] = useState(0);
  const observed = useRef<{
    root: HTMLElement | null;
    obs: MutationObserver | null;
  }>({ root: null, obs: null });
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (observed.current.root === root) return;
    observed.current.obs?.disconnect();
    const obs =
      root && typeof MutationObserver !== "undefined"
        ? new MutationObserver(() => setDomVersion((v) => v + 1))
        : null;
    obs?.observe(root!, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    observed.current = { root, obs };
    // Neues Wurzelelement (Ansicht neu aufgebaut): neu zuordnen und malen.
    setDomVersion((v) => v + 1);
  });
  useLayoutEffect(() => () => observed.current.obs?.disconnect(), []);

  // Zeichenstrom je DOM-Stand, Fundstellen je Satznummer dieser Sitzung und dieses
  // Stroms: der nächste Satz wird hinter dem vorigen gesucht, gleichlautende Sätze
  // treffen so die richtige Stelle.
  const found = useRef<{
    token: number | null;
    root: HTMLElement | null;
    version: number;
    content: unknown;
    stream: CharStream | null;
    spans: Map<number, MatchSpan>;
  }>({
    token: null,
    root: null,
    version: -1,
    content: null,
    stream: null,
    spans: new Map(),
  });
  const own = useRef<Highlight | null>(null);
  const shown = useRef<string | null>(null);

  const clear = () => {
    if (own.current && CSS.highlights.get(READ_ALOUD_HIGHLIGHT) === own.current)
      CSS.highlights.delete(READ_ALOUD_HIGHLIGHT);
    own.current = null;
    shown.current = null;
  };

  useLayoutEffect(() => {
    if (!canPaintHighlights()) return;
    const root = rootRef.current;
    if (!active || !root) {
      clear();
      return;
    }
    // Nach einem Satzsprung ist der neue Satz kurz unbekannt: die alte Markierung
    // bleibt stehen, bis er gemeldet wird -- kein Flackern.
    if (index === null || text === null) return;

    let cache = found.current;
    if (
      cache.token !== token ||
      cache.root !== root ||
      cache.version !== domVersion ||
      cache.content !== content ||
      !cache.stream
    ) {
      cache = {
        token,
        root,
        version: domVersion,
        content,
        stream: buildCharStream(root),
        spans: new Map(),
      };
      found.current = cache;
    }
    const stream = cache.stream!;
    const spans = cache.spans;

    let span = spans.get(index) ?? null;
    if (!span) {
      let from = 0;
      let prev = -1;
      for (const [i, known] of spans) {
        if (i < index && i > prev) {
          prev = i;
          from = known.end;
        }
      }
      span = locateSentence(stream, normalizeForMatch(text), from);
    }
    if (!span) {
      clear();
      return;
    }
    spans.set(index, span);

    const ranges = rangesOfSpan(stream, span);
    if (!ranges.length) {
      clear();
      return;
    }
    const h = new Highlight(...ranges);
    CSS.highlights.set(READ_ALOUD_HIGHLIGHT, h);
    own.current = h;
    // Ins Bild holen bei jedem neuen Satz, neuen Lauf (Neustart) und neuen Aufbau der Ansicht.
    const shownKey = `${token}:${index}`;
    if (shown.current !== shownKey) {
      shown.current = shownKey;
      revealRange(ranges[0]);
    }
  }, [active, token, index, text, content, domVersion, rootRef]);

  useLayoutEffect(
    () => () => {
      if (canPaintHighlights()) clear();
    },
    [],
  );
}
