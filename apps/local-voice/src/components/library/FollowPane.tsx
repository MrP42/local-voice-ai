import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ArrowDownToLine, Crosshair, X } from "lucide-react";
import { IconButton } from "../ui/IconButton";
import type { AudioSyncInsert } from "@/bindings";
import type { TextParagraph } from "@/lib/library/follow";
import type { Span } from "@/lib/library/sentences";

/** So lange pausiert das Mitscrollen nach einem Scrollen des Nutzers. */
export const FOLLOW_PAUSE_MS = 5000;

/** Name der Hervorhebung (CSS Custom Highlight API) für den gehörten Satz. */
const HL_SENTENCE = "follow-cur";

/**
 * Satz ohne eigene Spans hervorheben: ein Span, der mit jedem Satz wandert,
 * würde den Umbruch ändern (siehe Reader, lva#232). Ohne Highlight-API bleibt
 * es beim hervorgehobenen Absatz.
 */
const canPaint = () =>
  typeof CSS !== "undefined" &&
  "highlights" in CSS &&
  typeof Highlight !== "undefined";

/** Beginn des sichtbaren Absatztexts im Buch (Überschriften ohne „# “). */
export const textStartOf = (p: TextParagraph) =>
  p.heading ? p.end - p.text.length : p.start;

/** Textknoten eines Absatzes mit ihrer Buchstelle (Einschübe zählen nicht). */
function textNodes(el: HTMLElement): { start: number; node: Text }[] {
  const out: { start: number; node: Text }[] = [];
  for (const piece of el.querySelectorAll<HTMLElement>("[data-start]")) {
    const node = piece.firstChild;
    if (node?.nodeType === Node.TEXT_NODE)
      out.push({ start: Number(piece.dataset.start), node: node as Text });
  }
  return out;
}

/** Buchstelle unter einem Klick (null, wenn nicht ermittelbar). */
function charsAtPoint(el: HTMLElement, x: number, y: number): number | null {
  const doc = document as Document & {
    caretPositionFromPoint?(
      x: number,
      y: number,
    ): { offsetNode: Node; offset: number } | null;
    caretRangeFromPoint?(x: number, y: number): Range | null;
  };
  let node: Node | null = null;
  let offset = 0;
  const pos = doc.caretPositionFromPoint?.(x, y);
  if (pos) {
    node = pos.offsetNode;
    offset = pos.offset;
  } else {
    const r = doc.caretRangeFromPoint?.(x, y);
    if (r) {
      node = r.startContainer;
      offset = r.startOffset;
    }
  }
  if (!node || !el.contains(node)) return null;
  const hit = textNodes(el).find((e) => e.node === node);
  return hit ? hit.start + offset : null;
}

type Piece =
  | { kind: "text"; start: number; text: string }
  | { kind: "insert"; key: string; text: string };

/** Absatztext mit den Einschüben aus dem Hörbuch an ihrer Stelle. */
function piecesOf(p: TextParagraph, inserts: AudioSyncInsert[]): Piece[] {
  const start = textStartOf(p);
  const end = start + p.text.length;
  // Die Einschübe sind schon diesem Absatz zugeordnet; was am Ende oder
  // zwischen zwei Absätzen liegt, hängt hinten an (vor dem ersten: vorn).
  const mine = inserts.map((i) =>
    i.at > end ? { ...i, at: end } : i.at < start ? { ...i, at: start } : i,
  );
  if (!mine.length) return [{ kind: "text", start, text: p.text }];
  const out: Piece[] = [];
  let at = start;
  for (const ins of [...mine].sort((a, b) => a.at - b.at)) {
    if (ins.at > at) {
      out.push({
        kind: "text",
        start: at,
        text: p.text.slice(at - start, ins.at - start),
      });
      at = ins.at;
    }
    out.push({
      kind: "insert",
      key: `${ins.at}:${ins.start_ms}`,
      text: ins.text,
    });
  }
  if (at < end)
    out.push({ kind: "text", start: at, text: p.text.slice(at - start) });
  return out;
}

const NO_INSERTS: AudioSyncInsert[] = [];

const Paragraph = React.memo(
  ({
    p,
    index,
    active,
    inserts,
    onPick,
  }: {
    p: TextParagraph;
    index: number;
    active: boolean;
    inserts: AudioSyncInsert[];
    onPick(index: number, chars: number | null): void;
  }) => {
    const { t } = useTranslation();
    const pieces = piecesOf(p, inserts);
    return (
      <p
        className={`library-follow__p${p.heading ? " library-follow__p--head" : ""}${active ? " library-follow__p--now" : ""}`}
        data-index={index}
        data-active={active ? "true" : undefined}
        aria-current={active ? "location" : undefined}
        onClick={(e) =>
          onPick(index, charsAtPoint(e.currentTarget, e.clientX, e.clientY))
        }
      >
        {pieces.map((piece) =>
          piece.kind === "text" ? (
            <span key={piece.start} data-start={piece.start}>
              {piece.text}
            </span>
          ) : (
            <ins
              key={piece.key}
              className="library-follow__ins"
              title={t("library.follow.insert")}
              data-testid="library-follow-insert"
            >
              <span className="sr-only">{t("library.follow.insert")}: </span>
              {piece.text}
            </ins>
          ),
        )}
      </p>
    );
  },
);
Paragraph.displayName = "Paragraph";

/**
 * Lesefläche zum Mitlesen (lva#204, Hörbuch-Sync): der Text des Buchs, der
 * gerade gehörte Satz hervorgehoben und mitgescrollt. Text, der nur im
 * Hörbuch vorkommt, steht farbig an seiner Stelle. Ein Scrollen des Nutzers
 * hält das Mitscrollen {@link FOLLOW_PAUSE_MS} an; danach erscheint „Zur
 * aktuellen Stelle“. Ein Klick auf einen Satz springt im Hörbuch dorthin –
 * im Korrekturmodus legt er stattdessen fest, dass dieser Satz gerade zu
 * hören ist.
 */
export const FollowPane: React.FC<{
  title: string;
  paragraphs: TextParagraph[];
  active: number;
  /** Gerade gehörter Satz (Buchstellen). */
  sentence: Span | null;
  /** Gehörter Text ohne Gegenstück im Buch. */
  inserts: AudioSyncInsert[];
  /** Stelle nur geschätzt (Text und Hörbuch ohne gemeinsame Zeitachse). */
  approx: boolean;
  /** Startstelle wird im Hörbuch gesucht. */
  searching?: boolean;
  /** Klick auf eine Stelle: dorthin springen. */
  onPick(chars: number): void;
  /** Korrektur: diese Stelle ist gerade zu hören (fehlt bei Transkript). */
  onAnchor?(chars: number): void;
  /** Satz zur Buchstelle (für Klick und Korrektur). */
  sentenceOf(chars: number): Span | null;
  onClose(): void;
}> = ({
  title,
  paragraphs,
  active,
  sentence,
  inserts,
  approx,
  searching = false,
  onPick,
  onAnchor,
  sentenceOf,
  onClose,
}) => {
  const { t } = useTranslation();
  const box = useRef<HTMLDivElement>(null);
  const [away, setAway] = useState(false);
  const [correcting, setCorrecting] = useState(false);
  const pausedUntil = useRef(0);
  const timer = useRef<number | null>(null);
  const lastActive = useRef(-1);
  const lastSentence = useRef(-1);

  /** Einschübe je Absatz (gleiche leere Liste, wenn keiner: memo greift). */
  const insertsOf = useMemo(() => {
    const map = new Map<number, AudioSyncInsert[]>();
    if (!inserts.length) return map;
    const starts = paragraphs.map(textStartOf);
    for (const ins of inserts) {
      let lo = 0;
      let hi = paragraphs.length - 1;
      let found = -1;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (starts[mid] <= ins.at) {
          found = mid;
          lo = mid + 1;
        } else hi = mid - 1;
      }
      if (found < 0) found = 0;
      const list = map.get(found) ?? [];
      list.push(ins);
      map.set(found, list);
    }
    return map;
  }, [inserts, paragraphs]);

  /** Element oder Range der aktuellen Stelle (Satz, sonst Absatz). */
  const scrollToActive = useCallback((smooth: boolean) => {
    const container = box.current;
    const el = container?.querySelector<HTMLElement>('[data-active="true"]');
    if (!el || !container) return;
    let top = el.offsetTop - container.clientHeight / 2 + el.offsetHeight / 2;
    // Lange Absätze: auf den Satz zielen, nicht auf die Absatzmitte.
    const hl = canPaint() ? CSS.highlights.get(HL_SENTENCE) : undefined;
    const first = hl
      ? (hl.values().next().value as Range | undefined)
      : undefined;
    if (first && el.offsetHeight > container.clientHeight / 2) {
      const r = first.getBoundingClientRect();
      const c = container.getBoundingClientRect();
      top = container.scrollTop + (r.top - c.top) - container.clientHeight / 3;
    }
    container.scrollTo({
      top: Math.max(0, top),
      behavior: smooth ? "smooth" : "auto",
    });
  }, []);

  // Satz malen (ohne Layoutwechsel), bevor gescrollt wird.
  useLayoutEffect(() => {
    if (!canPaint()) return;
    const el = box.current?.querySelector<HTMLElement>('[data-active="true"]');
    if (!el || !sentence) {
      CSS.highlights.delete(HL_SENTENCE);
      return;
    }
    const ranges: Range[] = [];
    for (const e of textNodes(el)) {
      const from = Math.max(0, sentence.start - e.start);
      const to = Math.min(e.node.length, sentence.end - e.start);
      if (to <= from) continue;
      const r = document.createRange();
      r.setStart(e.node, from);
      r.setEnd(e.node, to);
      ranges.push(r);
    }
    CSS.highlights.set(HL_SENTENCE, new Highlight(...ranges));
  }, [sentence?.start, sentence?.end, active, insertsOf]);

  useLayoutEffect(
    () => () => {
      if (canPaint()) CSS.highlights.delete(HL_SENTENCE);
    },
    [],
  );

  // Mitscrollen, solange der Nutzer nicht gerade selbst scrollt.
  const sentenceStart = sentence?.start ?? -1;
  useEffect(() => {
    if (
      active === lastActive.current &&
      sentenceStart === lastSentence.current &&
      !away
    )
      return;
    lastActive.current = active;
    lastSentence.current = sentenceStart;
    if (Date.now() < pausedUntil.current || away) return;
    scrollToActive(true);
  }, [active, sentenceStart, away, scrollToActive]);

  // Beim Öffnen sofort an die Stelle, ohne Fahrt durch den ganzen Text.
  useEffect(() => {
    scrollToActive(false);
  }, [scrollToActive]);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const userScrolled = () => {
    pausedUntil.current = Date.now() + FOLLOW_PAUSE_MS;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setAway(true);
    }, FOLLOW_PAUSE_MS);
  };

  const backToNow = () => {
    pausedUntil.current = 0;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    setAway(false);
    scrollToActive(true);
  };

  // Neueste Werte für den Klick, damit die Absätze nicht bei jedem
  // Zeitschritt neu gezeichnet werden (React.memo).
  const latest = useRef({
    paragraphs,
    sentenceOf,
    correcting,
    onAnchor,
    onPick,
  });
  latest.current = { paragraphs, sentenceOf, correcting, onAnchor, onPick };
  const pick = useCallback((index: number, chars: number | null) => {
    const l = latest.current;
    pausedUntil.current = 0;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    setAway(false);
    const p = l.paragraphs[index];
    const at = chars ?? textStartOf(p);
    const s = l.sentenceOf(at);
    const target = s ? s.start : at;
    if (l.correcting && l.onAnchor) {
      setCorrecting(false);
      l.onAnchor(target);
    } else l.onPick(chars === null ? p.start : target);
  }, []);

  // Die Liste nur bei geändertem Absatz neu aufbauen, nicht bei jedem
  // Zeitschritt der Wiedergabe (Bücher mit Tausenden Absätzen).
  const list = useMemo(
    () =>
      paragraphs.map((p, i) => (
        <Paragraph
          key={p.start}
          p={p}
          index={i}
          active={i === active}
          inserts={insertsOf.get(i) ?? NO_INSERTS}
          onPick={pick}
        />
      )),
    [paragraphs, active, insertsOf, pick],
  );

  return (
    <div
      className="library-follow"
      role="region"
      aria-label={t("library.follow.region", { title })}
      data-testid="library-follow"
      data-correcting={correcting ? "true" : undefined}
    >
      <div className="library-follow__head">
        <strong className="library-follow__title">{title}</strong>
        {searching ? (
          <span
            className="library-meta"
            role="status"
            data-testid="library-follow-searching"
          >
            {t("library.follow.searching")}
          </span>
        ) : (
          approx && (
            <span className="library-meta" data-testid="library-follow-approx">
              {t("library.follow.approx")}
            </span>
          )
        )}
        <span className="library-bar__spacer" />
        {onAnchor && (
          <IconButton
            icon={<Crosshair size={18} aria-hidden="true" />}
            label={t("library.follow.correct")}
            onClick={() => setCorrecting((v) => !v)}
            aria-pressed={correcting}
            variant={correcting ? "secondary" : "ghost"}
            data-testid="library-follow-correct"
          />
        )}
        <IconButton
          icon={<X size={18} aria-hidden="true" />}
          label={t("library.follow.close")}
          onClick={onClose}
          data-testid="library-follow-close"
        />
      </div>
      {correcting && (
        <div
          className="library-follow__hint"
          role="status"
          data-testid="library-follow-correct-hint"
        >
          {t("library.follow.correctHint")}
        </div>
      )}
      <div
        className="library-follow__text"
        ref={box}
        // Nur echte Eingaben des Nutzers zählen, nicht das eigene Mitscrollen.
        onWheel={userScrolled}
        onTouchMove={userScrolled}
        onPointerDown={(e) => {
          // Auf die Bildlaufleiste getippt (Ziel ist der Behälter selbst).
          if (e.target === e.currentTarget) userScrolled();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && correcting) {
            setCorrecting(false);
            return;
          }
          if (
            [
              "ArrowUp",
              "ArrowDown",
              "PageUp",
              "PageDown",
              "Home",
              "End",
              " ",
            ].includes(e.key)
          )
            userScrolled();
        }}
        tabIndex={0}
        lang="de"
        data-testid="library-follow-text"
      >
        {list}
      </div>
      {away && (
        <button
          type="button"
          className="library-follow__now"
          onClick={backToNow}
          data-testid="library-follow-now"
        >
          <ArrowDownToLine size={16} aria-hidden="true" />
          {t("library.follow.current")}
        </button>
      )}
    </div>
  );
};
