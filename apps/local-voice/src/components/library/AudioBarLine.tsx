import React, {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import type { AudioSyncInsert } from "@/bindings";
import { paragraphAt, type TextParagraph } from "@/lib/library/follow";
import { sentencesOf } from "@/lib/library/sentences";
import {
  restartAtCurrent,
  stepPassage,
  unitsOf,
  type Passage,
  type Unit,
} from "@/lib/library/passage";
import { textStartOf } from "./FollowPane";

/** Einschübe kürzer als das blenden nicht um (sonst flackert die Zeile). */
const MIN_INSERT_MS = 1500;
/** Zeichenbreite im Verhältnis zur Schriftgröße (Segoe UI, Mittelwert). */
const CHAR_EM = 0.46;
/** Ab dieser Breite reichen zwei Zeilen, darunter drei. */
const WIDE_PX = 900;

interface Props {
  text: string;
  paragraphs: TextParagraph[];
  /** Gehörte Stelle (Zeichen). */
  chars: number;
  /** Gehörte Zeit (ms) – für Einschübe. */
  ms: number;
  /** Zählt Sprünge im Hörbuch: dann darf die Anzeige sofort folgen. */
  seekGen: number;
  inserts: AudioSyncInsert[];
  /** Stelle nur geschätzt (weder Transkript noch Abgleich). */
  approx: boolean;
  /** Abgleich nicht verfügbar: die Schätzung ist alles, was es gibt. */
  unavailable: boolean;
  searching: boolean;
  /** Hörbuch läuft oder ein Fenster wird gerade erkannt: der Abgleich arbeitet. */
  aligningNow: boolean;
  onOpen(): void;
}

/**
 * Ruhige Textzeile über der Hörleiste: zwei bis drei Zeilen Buchtext, die
 * still stehen, solange das Hören darin liegt; der gehörte Satz ist markiert.
 * Ist die Stelle nur geschätzt und läuft der Abgleich noch, steht dort ein
 * Hinweis statt Text, der womöglich gar nicht gelesen wird.
 */
export const AudioBarLine: React.FC<Props> = ({
  text,
  paragraphs,
  chars,
  ms,
  seekGen,
  inserts,
  approx,
  unavailable,
  searching,
  aligningNow,
  onOpen,
}) => {
  const { t } = useTranslation();
  const box = useRef<HTMLButtonElement>(null);
  const nowEl = useRef<HTMLSpanElement>(null);

  // Platz: Zeilen und Zeichen je Zeile aus der Breite.
  const [geom, setGeom] = useState({ lines: 2, budget: 220 });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => {
      const cs = getComputedStyle(el);
      const w =
        el.clientWidth -
        parseFloat(cs.paddingLeft || "0") -
        parseFloat(cs.paddingRight || "0") -
        24;
      const font = parseFloat(cs.fontSize || "15") || 15;
      const lines = el.clientWidth >= WIDE_PX ? 2 : 3;
      const perLine = Math.max(20, w / (font * CHAR_EM));
      // Etwas Luft für Umbrüche an Wortgrenzen.
      const budget = Math.round(lines * perLine * 0.95);
      setGeom((g) =>
        g.lines === lines && Math.abs(g.budget - budget) < 8
          ? g
          : { lines, budget },
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const { lines, budget } = geom;

  /** Einheiten je Absatz, erst bei Bedarf zerlegt. */
  const unitCache = useMemo(
    () => new Map<number, Unit[]>(),
    [paragraphs, text, budget],
  );
  const unitsFor = useCallback(
    (i: number): Unit[] => {
      let list = unitCache.get(i);
      if (!list) {
        const p = paragraphs[i];
        // Lange Sätze kleiner teilen als der Platz: der gehörte Teil bleibt sichtbar.
        list = unitsOf(text, sentencesOf(p.text, textStartOf(p)), budget * 0.8);
        unitCache.set(i, list);
      }
      return list;
    },
    [paragraphs, text, budget, unitCache],
  );

  const para = paragraphs.length ? paragraphAt(paragraphs, chars) : -1;
  const units = para >= 0 ? unitsFor(para) : [];

  // Zustand der Zeile: Abschnitt bleibt stehen, nur die Markierung wandert.
  const state = useRef<{
    passage: Passage | null;
    seekGen: number;
    budget: number;
  }>({ passage: null, seekGen, budget });
  const [, redraw] = useState(0);
  const s = state.current;
  const seek = s.seekGen !== seekGen || s.budget !== budget;
  const passage = stepPassage(seek ? null : s.passage, {
    para,
    units,
    chars,
    budget,
    seek,
    now: performance.now(),
  });
  s.passage = passage;
  s.seekGen = seekGen;
  s.budget = budget;

  // Gehörter Satz außerhalb der sichtbaren Zeilen: ab ihm neu beginnen.
  useLayoutEffect(() => {
    const el = nowEl.current;
    const host = box.current;
    const p = state.current.passage;
    if (!el || !host || !p || p.cur === 0) return;
    const clip = el.parentElement;
    if (!clip) return;
    const bottom = el.offsetTop + el.offsetHeight;
    if (bottom <= clip.clientHeight + 2) return;
    state.current.passage = restartAtCurrent(p, unitsFor(p.para), budget);
    redraw((n) => n + 1);
  });

  // Ein Rücksprung wartet auf HOLD_BACK_MS: auch ohne neue Zeit nachziehen.
  const behind = passage?.behindSince != null;
  useLayoutEffect(() => {
    if (!behind) return;
    const timer = window.setTimeout(() => redraw((n) => n + 1), 1600);
    return () => window.clearTimeout(timer);
  }, [behind, passage]);

  const insert = inserts.find(
    (i) =>
      ms >= i.start_ms &&
      ms < i.end_ms &&
      i.end_ms - i.start_ms >= MIN_INSERT_MS &&
      i.text.trim(),
  );
  const aligning = approx && !unavailable && aligningNow && !searching;

  const passageText = passage
    ? passage.units.map((u) => text.slice(u.start, u.end)).join(" ")
    : "";
  const label = searching
    ? t("library.follow.searching")
    : aligning
      ? t("library.follow.aligning")
      : (insert?.text ?? passageText);

  let content: React.ReactNode;
  if (searching || aligning) {
    content = <span className="library-meta">{label}</span>;
  } else if (insert) {
    content = (
      <ins className="library-follow__ins" key={`i${insert.start_ms}`}>
        {insert.text}
      </ins>
    );
  } else if (passage) {
    content = (
      <span
        className="library-audiobar__passage"
        key={`${passage.para}:${passage.from}`}
        data-testid="library-audiobar-passage"
      >
        {passage.units.map((u, i) => (
          <React.Fragment key={u.start}>
            {i > 0 && " "}
            <span
              ref={i === passage.cur ? nowEl : undefined}
              className={
                i < passage.cur
                  ? "library-audiobar__done"
                  : i === passage.cur
                    ? "library-audiobar__sentence"
                    : "library-audiobar__next"
              }
              data-testid={
                i === passage.cur ? "library-audiobar-now" : undefined
              }
            >
              {text.slice(u.start, u.end)}
            </span>
          </React.Fragment>
        ))}
      </span>
    );
  } else content = null;

  return (
    <button
      ref={box}
      type="button"
      className="library-audiobar__line"
      style={{ "--line-rows": lines } as React.CSSProperties}
      onClick={onOpen}
      aria-label={t("library.follow.lineLabel", { text: label })}
      title={searching || aligning ? undefined : label}
      data-testid="library-audiobar-line"
    >
      {approx && !aligning && !searching && passage && !insert && (
        <span
          className="library-audiobar__approx"
          title={t("library.follow.approx")}
          data-testid="library-audiobar-line-approx"
        >
          ≈
        </span>
      )}
      <span className="library-audiobar__clip">{content}</span>
    </button>
  );
};
