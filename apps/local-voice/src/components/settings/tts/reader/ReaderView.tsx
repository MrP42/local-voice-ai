import React, {
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight, LocateFixed } from "lucide-react";
import type { SpeakerRef } from "@/lib/voices/speakerMarkers";
import { providerSpeakerColor } from "../speakers/speakerProvider";
import {
  parseReaderDoc,
  type ReaderBlock,
  type ReaderRun,
} from "@/lib/reader/readerDoc";
import type { SourceRange } from "@/lib/reader/sentenceLocator";
import { useSentenceClock, type SentenceClock } from "./useSentenceClock";
import "./reader.css";

/** Eine markierte Stelle (Befund) im Seitentext. */
export interface ReaderMark extends SourceRange {
  selected?: boolean;
}

export interface ReaderPageInfo {
  /** Erste sichtbare Seite, 0-basiert. */
  page: number;
  pages: number;
  /** 1 oder 2 (aufgeschlagenes Buch). */
  perView: number;
  /** Seite des gerade gelesenen Satzes, 0-basiert; null ohne Wiedergabe. */
  currentPage: number | null;
}

export interface ReaderViewApi {
  goToPage(page: number): void;
  /** Die Seite zeigen, auf der dieser Offset steht. */
  revealOffset(offset: number): void;
  /** Erste Quelle-Position, die auf dieser Seite sichtbar ist. */
  offsetOfPage(page: number): number | null;
  /** Seite (0-basiert), auf der dieser Offset steht. */
  pageOfOffset(offset: number): number | null;
}

interface ReaderViewProps {
  text: string;
  speakers: SpeakerRef[];
  lang?: string;
  current: SourceRange | null;
  /** Laufender Satz: daraus die Stelle, die gerade zu hören ist (Wort). */
  clock?: SentenceClock | null;
  marks?: ReaderMark[];
  twoPages: boolean;
  fontSize: number;
  /** Leseschrift (book | sans | wide). */
  fontFamily?: string;
  showSpeakers: boolean;
  /** Seite folgt dem gelesenen Satz. Blättert der Nutzer selbst, endet das. */
  follow: boolean;
  onFollowChange(follow: boolean): void;
  /** Klick in den Text: ab dieser Stelle vorlesen. */
  onActivate(offset: number): void;
  onSpeakerMenu?(name: string, anchor: HTMLElement): void;
  onPageInfo?(info: ReaderPageInfo): void;
  apiRef?: React.Ref<ReaderViewApi>;
  title?: string;
}

/** Mindestbreite des Textes einer Seite, ab der zwei Seiten nebeneinander stehen. */
const MIN_TWO_PAGE_TEXT = 340;
const PAGE_PAD = 52;
const PAGE_PAD_NARROW = 16;
const NARROW = 560;

/** Textknoten und Zeichen unter dem Mauszeiger (WebView2 und WebKit kennen
 *  `caretRangeFromPoint`, Firefox nur `caretPositionFromPoint`). */
function caretAt(x: number, y: number): { node: Node; offset: number } | null {
  const doc = document as Document & {
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
  };
  if (typeof doc.caretRangeFromPoint === "function") {
    const r = doc.caretRangeFromPoint(x, y);
    return r ? { node: r.startContainer, offset: r.startOffset } : null;
  }
  const p = doc.caretPositionFromPoint?.(x, y);
  return p ? { node: p.offsetNode, offset: p.offset } : null;
}

type Piece = {
  text: string;
  start: number;
  cur: boolean;
  word: boolean;
  mark?: ReaderMark;
};

/** Das Wort, in dem `at` liegt (bis zum nächsten Leerraum). */
export function wordAt(text: string, at: number, within: SourceRange): SourceRange {
  let start = Math.max(within.start, Math.min(at, within.end - 1));
  // Auf Leerraum gelandet: zum nächsten Wort vor.
  while (start < within.end - 1 && /\s/.test(text[start])) start++;
  while (start > within.start && !/\s/.test(text[start - 1])) start--;
  let end = start;
  while (end < within.end && !/\s/.test(text[end])) end++;
  return { start, end: Math.max(end, start + 1) };
}

/** Seitenstand je Text, damit die Ansicht beim Zurückkehren dort steht, wo
 *  sie war — ohne Umblättern von Seite 1 her. */
const lastPages = new Map<string, number>();

/** Einen Lauf an den Grenzen von Hervorhebung und Befunden zerteilen. */
function piecesOf(
  run: ReaderRun,
  current: SourceRange | null,
  marks: ReaderMark[],
  word: SourceRange | null,
): Piece[] {
  const cuts = new Set<number>([run.start, run.end]);
  const inside = (o: number) => o > run.start && o < run.end;
  for (const r of [current, word]) {
    if (!r) continue;
    if (inside(r.start)) cuts.add(r.start);
    if (inside(r.end)) cuts.add(r.end);
  }
  for (const m of marks) {
    if (m.end <= run.start || m.start >= run.end) continue;
    if (inside(m.start)) cuts.add(m.start);
    if (inside(m.end)) cuts.add(m.end);
  }
  const sorted = [...cuts].sort((a, b) => a - b);
  const out: Piece[] = [];
  for (let i = 0; i < sorted.length - 1; i++) {
    const a = sorted[i];
    const b = sorted[i + 1];
    out.push({
      text: run.text.slice(a - run.start, b - run.start),
      start: a,
      cur: !!current && a >= current.start && b <= current.end,
      word: !!word && a >= word.start && b <= word.end,
      mark: marks.find((m) => a >= m.start && b <= m.end),
    });
  }
  return out;
}

const Runs: React.FC<{
  block: ReaderBlock;
  current: SourceRange | null;
  marks: ReaderMark[];
  word: SourceRange | null;
}> = ({ block, current, marks, word }) => {
  const { t } = useTranslation();
  return (
    <>
      {block.runs.map((run) => {
        if (run.page !== undefined)
          return (
            <span
              key={run.start}
              className="reader-pagemark"
              role="note"
              aria-label={t("tts.reader.originalPage", { page: run.page })}
              title={t("tts.reader.originalPage", { page: run.page })}
            >
              {run.page}
            </span>
          );
        if (run.tag) return null;
        const pieces = piecesOf(run, current, marks, word);
        const inner = pieces.map((p) => {
          const cls = [
            p.cur ? "reader-cur" : "",
            p.word ? "reader-word" : "",
            p.mark ? "reader-mark" : "",
            p.mark?.selected ? "reader-mark--sel" : "",
          ]
            .filter(Boolean)
            .join(" ");
          return (
            <span
              key={p.start}
              data-start={p.start}
              className={cls || undefined}
              data-cur={p.cur ? "" : undefined}
              data-word={p.word ? "" : undefined}
              data-mark={p.mark?.selected ? "" : undefined}
            >
              {p.text}
            </span>
          );
        });
        if (run.bold && run.italic)
          return (
            <strong key={run.start}>
              <em>{inner}</em>
            </strong>
          );
        if (run.bold) return <strong key={run.start}>{inner}</strong>;
        if (run.italic) return <em key={run.start}>{inner}</em>;
        return <React.Fragment key={run.start}>{inner}</React.Fragment>;
      })}
    </>
  );
};

/**
 * Buchansicht des Vorlesetexts (lva#148): Seiten statt Endlos-Scroll, ein- oder
 * zweiseitig, Sprecher am Rand, der gelesene Satz hervorgehoben.
 *
 * Seiten entstehen per CSS-Spaltenumbruch: der Text fließt in Spalten von
 * Seitenbreite, Blättern verschiebt den Fluss um eine Spalte. Das ist reine
 * Darstellung — die Wiedergabe merkt davon nichts, Blättern unterbricht nie
 * das Audio.
 */
export const ReaderView: React.FC<ReaderViewProps> = ({
  text,
  speakers,
  lang,
  current,
  clock = null,
  marks = [],
  twoPages,
  fontSize,
  fontFamily = "book",
  showSpeakers,
  follow,
  onFollowChange,
  onActivate,
  onSpeakerMenu,
  onPageInfo,
  apiRef,
  title,
}) => {
  const { t } = useTranslation();
  const blocks = useMemo(() => parseReaderDoc(text, speakers), [text, speakers]);
  const bookRef = useRef<HTMLDivElement>(null);
  const flowRef = useRef<HTMLDivElement>(null);
  const [geo, setGeo] = useState({
    colW: 0,
    gap: 0,
    pad: PAGE_PAD,
    perView: 1,
    pages: 1,
    narrow: false,
  });
  const pageKey = `${title ?? ""}:${text.length}`;
  const [page, setPageRaw] = useState(() => lastPages.get(pageKey) ?? 0);
  // Animiert wird nur echtes Blättern (Knopf, Taste, Wischen, Mitblättern) —
  // nicht das Wiederherstellen nach einem App-Wechsel oder ein neuer Umbruch
  // nach einer Einstellung.
  const [animate, setAnimate] = useState(false);
  const setPage = setPageRaw;
  const turnTo = useCallback((next: React.SetStateAction<number>) => {
    setAnimate(true);
    setPageRaw(next);
  }, []);
  useEffect(() => {
    if (!animate) return;
    const id = window.setTimeout(() => setAnimate(false), 400);
    return () => window.clearTimeout(id);
  }, [animate, page]);
  // Text oder Titel kommen oft erst nach dem ersten Rendern: dann den
  // gemerkten Stand dieses Texts holen (ohne Animation).
  const pageKeyRef = useRef(pageKey);
  useLayoutEffect(() => {
    if (pageKeyRef.current === pageKey) return;
    pageKeyRef.current = pageKey;
    const saved = lastPages.get(pageKey);
    if (saved !== undefined) setPageRaw(saved);
  }, [pageKey]);
  useEffect(() => {
    lastPages.set(pageKeyRef.current, page);
  }, [page]);
  const step = geo.colW + geo.gap;

  // Die gerade hörbare Stelle: Anteil des Satzes nach Zeit, auf das Wort
  // gerundet. Ohne Satzdauer (ältere Engine) bleibt es beim Satz.
  const within = useSentenceClock(current ? clock : null);
  const word = useMemo(() => {
    if (!current || !clock || current.end <= current.start) return null;
    const at = current.start + Math.floor(within * (current.end - current.start));
    return wordAt(text, at, current);
  }, [current, clock, within, text]);

  const pageOfElement = useCallback(
    (el: Element | null): number | null => {
      const flow = flowRef.current;
      if (!el || !flow || step <= 0) return null;
      // Beide Rechtecke tragen die Verschiebung; die Differenz ist die Lage im Fluss.
      // Erstes Rechteck: ein über die Seiten umbrochenes Wort beginnt dort.
      const rect = el.getClientRects()[0] ?? el.getBoundingClientRect();
      const x = rect.left - flow.getBoundingClientRect().left;
      return Math.max(0, Math.min(geo.pages - 1, Math.floor((x + 2) / step)));
    },
    [step, geo.pages],
  );

  const elementAt = useCallback((offset: number): Element | null => {
    const flow = flowRef.current;
    if (!flow) return null;
    let best: Element | null = null;
    for (const el of flow.querySelectorAll("[data-start]")) {
      const start = Number((el as HTMLElement).dataset.start);
      if (start > offset) break;
      best = el;
    }
    return best;
  }, []);

  const clampPage = useCallback(
    (p: number) => {
      const max = Math.max(0, geo.pages - 1);
      const c = Math.max(0, Math.min(max, p));
      return c - (c % geo.perView);
    },
    [geo.pages, geo.perView],
  );

  // Seitengeometrie: aus der Breite der Buchfläche, nicht aus dem Inhalt —
  // sonst schaukeln sich Umbruch und Messung gegenseitig auf.
  const measure = useCallback(() => {
    const book = bookRef.current;
    const flow = flowRef.current;
    if (!book || !flow) return;
    const width = book.clientWidth;
    if (width <= 0) return;
    const narrow = width < NARROW;
    const pad = narrow ? PAGE_PAD_NARROW : PAGE_PAD;
    const gutter = showSpeakers && !narrow ? 96 : 0;
    const gap = pad * 2;
    const twoFit = (width - pad * 2 - gap) / 2 - gutter >= MIN_TWO_PAGE_TEXT;
    const perView = twoPages && !narrow && twoFit ? 2 : 1;
    const colW = perView === 2 ? (width - pad * 2 - gap) / 2 : width - pad * 2;
    flow.style.columnWidth = `${colW}px`;
    flow.style.columnGap = `${gap}px`;
    flow.style.width = `${perView === 2 ? colW * 2 + gap : colW}px`;
    const pages = Math.max(1, Math.round((flow.scrollWidth + gap) / (colW + gap)));
    setGeo({ colW, gap, pad, perView, pages, narrow });
  }, [twoPages, showSpeakers]);

  useLayoutEffect(() => {
    measure();
  }, [measure, blocks, fontSize, fontFamily]);

  useEffect(() => {
    const book = bookRef.current;
    if (!book || typeof ResizeObserver === "undefined") return;
    let last = book.clientWidth;
    let lastH = book.clientHeight;
    const ro = new ResizeObserver(() => {
      if (book.clientWidth === last && book.clientHeight === lastH) return;
      last = book.clientWidth;
      lastH = book.clientHeight;
      measure();
    });
    ro.observe(book);
    return () => ro.disconnect();
  }, [measure]);

  // Nach neuem Umbruch die gelesene Stelle wieder zeigen.
  useLayoutEffect(() => {
    // Vor der ersten Messung kennt die Ansicht nur eine Seite — ein
    // gemerkter Stand fiele sonst auf Seite 1 zurück.
    if (geo.colW <= 0) return;
    setPage((p) => clampPage(p));
  }, [geo, clampPage]);

  // Die hörbare Stelle: das Wort, sonst der Satzanfang.
  const curEl = () =>
    flowRef.current?.querySelector("[data-word]") ??
    flowRef.current?.querySelector("[data-cur]") ??
    null;

  // Mitblättern wortgenau: umgeblättert wird, wenn das Wort, das gerade zu
  // hören ist, auf der nächsten Seite steht — auch mitten im Satz.
  useLayoutEffect(() => {
    if (!follow || !current) return;
    const p = pageOfElement(curEl());
    if (p === null) return;
    if (p < page || p >= page + geo.perView) turnTo(clampPage(p));
  }, [current, word, follow, geo, pageOfElement, clampPage]);

  // Nach dem Commit messen: im Render stünde noch das DOM der Vorgängerfassung.
  const [currentPage, setCurrentPage] = useState<number | null>(null);
  useLayoutEffect(() => {
    setCurrentPage(current ? pageOfElement(curEl()) : null);
  }, [current, word, geo, pageOfElement]);
  useEffect(() => {
    onPageInfo?.({ page, pages: geo.pages, perView: geo.perView, currentPage });
  }, [page, geo.pages, geo.perView, currentPage, onPageInfo]);

  useImperativeHandle(
    apiRef,
    () => ({
      goToPage: (p: number) => turnTo(clampPage(p)),
      revealOffset: (offset: number) => {
        const p = pageOfElement(elementAt(offset));
        if (p !== null) setPage(clampPage(p));
      },
      pageOfOffset: (offset: number) => {
        const p = pageOfElement(elementAt(offset));
        return p === null ? null : clampPage(p);
      },
      offsetOfPage: (p: number) => {
        const flow = flowRef.current;
        if (!flow) return null;
        for (const el of flow.querySelectorAll("[data-start]")) {
          if (pageOfElement(el) === p) return Number((el as HTMLElement).dataset.start);
        }
        return null;
      },
    }),
    [clampPage, pageOfElement, elementAt, turnTo],
  );

  const turn = (dir: number) => {
    turnTo((p) => clampPage(p + dir * geo.perView));
    if (current) onFollowChange(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowRight" || e.key === "PageDown") {
      turn(1);
      e.preventDefault();
    } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
      turn(-1);
      e.preventDefault();
    } else if (e.key === "Home") {
      setPage(0);
      e.preventDefault();
    } else if (e.key === "End") {
      setPage(clampPage(geo.pages - 1));
      e.preventDefault();
    }
  };

  const onClick = (e: React.MouseEvent) => {
    const who = (e.target as HTMLElement).closest<HTMLElement>("[data-speaker]");
    if (who) {
      onSpeakerMenu?.(who.dataset.speaker ?? "", who);
      return;
    }
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) return; // Markieren zum Kopieren ist kein Sprung.
    const caret = caretAt(e.clientX, e.clientY);
    const node = caret?.node ?? null;
    const host = (
      node?.nodeType === Node.TEXT_NODE ? node.parentElement : (node as Element | null)
    )?.closest<HTMLElement>("[data-start]") ??
      (e.target as HTMLElement).closest<HTMLElement>("[data-start]");
    if (!host || !flowRef.current?.contains(host)) return;
    const within = node?.nodeType === Node.TEXT_NODE && host.contains(node) ? (caret?.offset ?? 0) : 0;
    const offset = Number(host.dataset.start) + within;
    onActivate(offset);
  };

  const touchX = useRef<number | null>(null);
  const lastPage = page + geo.perView >= geo.pages;
  const showFollow = !!current && currentPage !== null && (currentPage < page || currentPage >= page + geo.perView);
  const pageLabel =
    geo.perView === 2
      ? t("tts.reader.pagesShown", {
          first: page + 1,
          second: Math.min(geo.pages, page + 2),
          count: geo.pages,
        })
      : t("tts.reader.pageShown", { page: page + 1, count: geo.pages });

  return (
    <div className="reader-stage" data-testid="reader-stage">
      <div
        ref={bookRef}
        className={`reader-book${geo.perView === 2 ? " reader-book--two" : ""}${showSpeakers ? "" : " reader-book--nospeakers"}${geo.narrow ? " reader-book--narrow" : ""}`}
        style={{ "--reader-fs": `${fontSize}px` } as React.CSSProperties}
        data-testid="reader-book"
        data-font={fontFamily}
        data-pages={geo.pages}
        data-per-view={geo.perView}
        data-page={page}
      >
        {title && (
          <div className="reader-run" aria-hidden="true">
            <span>{title}</span>
          </div>
        )}
        <div
          ref={flowRef}
          className={`reader-flow${animate ? " reader-flow--turning" : ""}`}
          lang={lang}
          role="document"
          aria-label={t("tts.reader.label")}
          aria-roledescription={t("tts.reader.roleBook")}
          tabIndex={0}
          style={{
            left: `${geo.pad}px`,
            transform: `translateX(${-page * step}px)`,
          }}
          onKeyDown={onKeyDown}
          onClick={onClick}
          onFocus={(e) => {
            // Fokus auf einer verdeckten Seite (Tab auf einen Sprecher):
            // der Browser scrollt sonst den Rahmen zusätzlich zur Verschiebung.
            if (bookRef.current) bookRef.current.scrollLeft = 0;
            if (e.target === e.currentTarget) return;
            const p = pageOfElement(e.target);
            if (p !== null && (p < page || p >= page + geo.perView)) {
              setPage(clampPage(p));
              if (current) onFollowChange(false);
            }
          }}
          onTouchStart={(e) => (touchX.current = e.touches[0].clientX)}
          onTouchEnd={(e) => {
            if (touchX.current === null) return;
            const d = e.changedTouches[0].clientX - touchX.current;
            if (Math.abs(d) > 50) turn(d < 0 ? 1 : -1);
            touchX.current = null;
          }}
          data-testid="reader-flow"
        >
          {blocks.map((block) => {
            const key = `${block.start}:${block.kind}`;
            if (block.kind === "rule") return <hr key={key} />;
            if (block.kind === "heading") {
              const Tag = (block.level === 1 ? "h2" : block.level === 2 ? "h3" : "h4") as "h2";
              return (
                <Tag key={key} className={`reader-h${block.level ?? 1}`}>
                  <Runs block={block} current={current} marks={marks} word={word} />
                </Tag>
              );
            }
            const speaker = block.speaker;
            // Ohne Stimme trotzdem farbig (stabil je Name), damit man die
            // Sprecher im Buch auseinanderhält; der Strich ist dann gestrichelt.
            const color =
              speaker?.ref?.color ??
              (speaker ? providerSpeakerColor(speaker.name.toLowerCase()) : "var(--color-mid-gray)");
            const Tag = (block.kind === "quote" ? "blockquote" : block.kind === "item" ? "div" : "p") as "p";
            return (
              <Tag
                key={key}
                className={`reader-block reader-${block.kind}${block.continues ? " reader-cont" : ""}${speaker && !speaker.ref ? " reader-block--novoice" : ""}`}
                style={{ "--speaker-c": color } as React.CSSProperties}
              >
                {speaker && showSpeakers && (
                  <>
                    <span className="reader-bar" aria-hidden="true" />
                    <button
                      type="button"
                      className="reader-who"
                      data-speaker={speaker.name}
                      tabIndex={block.continues ? -1 : 0}
                      aria-label={t("tts.reader.speakerButton", { name: speaker.name })}
                      title={speaker.ref ? speaker.name : t("tts.reader.speakerUnknown", { name: speaker.name })}
                    >
                      {speaker.name}
                    </button>
                  </>
                )}
                {block.kind === "item" && <span className="reader-dot" aria-hidden="true">•</span>}
                <Runs block={block} current={current} marks={marks} word={word} />
              </Tag>
            );
          })}
        </div>
        <div className="reader-foot" aria-hidden="true">
          {geo.perView === 2 ? (
            <>
              <span>{page + 1}</span>
              <span>{Math.min(geo.pages, page + 2)}</span>
            </>
          ) : (
            <span>{t("tts.reader.pageOf", { page: page + 1, count: geo.pages })}</span>
          )}
        </div>
        <button
          type="button"
          className="reader-turn reader-turn--prev"
          onClick={() => turn(-1)}
          disabled={page === 0}
          aria-label={t("tts.reader.prevPage")}
          data-testid="reader-prev-page"
        >
          <ChevronLeft width={22} height={22} />
        </button>
        <button
          type="button"
          className="reader-turn reader-turn--next"
          onClick={() => turn(1)}
          disabled={lastPage}
          aria-label={t("tts.reader.nextPage")}
          data-testid="reader-next-page"
        >
          <ChevronRight width={22} height={22} />
        </button>
        {showFollow && (
          <button
            type="button"
            className="reader-follow"
            data-testid="reader-follow"
            onClick={() => {
              onFollowChange(true);
              if (currentPage !== null) setPage(clampPage(currentPage));
            }}
          >
            <LocateFixed width={14} height={14} aria-hidden="true" />
            {t("tts.reader.backToReading")}
          </button>
        )}
      </div>
      <p className="sr-only" aria-live="polite" data-testid="reader-page-label">
        {pageLabel}
      </p>
    </div>
  );
};
