import { useLayoutEffect, useRef, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import {
  buildCharStream,
  canPaintHighlights,
  locateSentence,
  normalizeForMatch,
  rangesOfSpan,
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
function revealRange(range: Range) {
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
  const sentence = mine?.sentence ?? null;
  const token = mine?.token ?? null;

  // Fundstellen je Satznummer dieser Sitzung und dieses Inhalts: der nächste Satz wird
  // hinter dem vorigen gesucht, gleichlautende Sätze treffen so die richtige Stelle.
  const found = useRef<{
    token: number | null;
    content: unknown;
    spans: Map<number, MatchSpan>;
  }>({ token: null, content: null, spans: new Map() });
  const own = useRef<Highlight | null>(null);
  const shownIndex = useRef<number | null>(null);

  const clear = () => {
    if (own.current && CSS.highlights.get(READ_ALOUD_HIGHLIGHT) === own.current)
      CSS.highlights.delete(READ_ALOUD_HIGHLIGHT);
    own.current = null;
    shownIndex.current = null;
  };

  useLayoutEffect(() => {
    if (!canPaintHighlights()) return;
    const root = rootRef.current;
    if (!mine || !root) {
      clear();
      return;
    }
    // Nach einem Satzsprung ist der neue Satz kurz unbekannt: die alte Markierung
    // bleibt stehen, bis er gemeldet wird -- kein Flackern.
    if (!sentence) return;

    const cache = found.current;
    if (cache.token !== token || cache.content !== content) {
      found.current = { token, content, spans: new Map() };
    }
    const spans = found.current.spans;

    const stream = buildCharStream(root);
    const wanted = normalizeForMatch(sentence.text);
    let from = 0;
    let prev = -1;
    for (const [i, span] of spans) {
      if (i < sentence.index && i > prev) {
        prev = i;
        from = span.end;
      }
    }
    const span =
      spans.get(sentence.index) ?? locateSentence(stream.chars, wanted, from);
    if (!span || span.end > stream.chars.length) {
      clear();
      return;
    }
    spans.set(sentence.index, span);

    const ranges = rangesOfSpan(stream, span);
    if (!ranges.length) {
      clear();
      return;
    }
    const h = new Highlight(...ranges);
    CSS.highlights.set(READ_ALOUD_HIGHLIGHT, h);
    own.current = h;
    if (shownIndex.current !== sentence.index) {
      shownIndex.current = sentence.index;
      revealRange(ranges[0]);
    }
  }, [mine, sentence, token, content, rootRef]);

  useLayoutEffect(
    () => () => {
      if (canPaintHighlights()) clear();
    },
    [],
  );
}
