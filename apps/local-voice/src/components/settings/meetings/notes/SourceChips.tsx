import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Presentation, Search } from "lucide-react";
import {
  formatAt,
  MAX_VISIBLE_SOURCES,
  splitSources,
} from "@/lib/meetingNotes";

interface SourceChipsProps {
  /** Segmentnummern (`source_segment_ids`) des Eintrags. */
  ids: number[];
  /** Startzeit eines Segments in ms; `null`, wenn es nicht (mehr) existiert. */
  startMsOf: (segmentIndex: number) => number | null;
  /** Neu transkribiert: die Nummern zeigen auf andere Stellen, Spruenge sind aus. */
  stale: boolean;
  onJump: (segmentIndex: number) => void;
  /** D5: Folien-Nummern (`source_slide_ids`) des Eintrags. */
  slideIds?: number[];
  /** D5: erste Sichtung einer Folie in ms; `null`, wenn es sie nicht (mehr) gibt. */
  slideStartMsOf?: (slideNumber: number) => number | null;
  /** D5: Sprung zur Folie (Reiter Folien, Abspielzeit). */
  onJumpSlide?: (slideNumber: number) => void;
}

/**
 * Quellen-Chips eines KI-Eintrags: die Zeit (`mm:ss`) der belegenden Stelle,
 * hoechstens drei, der Rest hinter "+n". Ein Klick springt ins Transkript. D5: Folien
 * als Belege (`F7`) stehen daneben; ein Klick fuehrt zur Folie und zu ihrer Zeit.
 */
export const SourceChips: React.FC<SourceChipsProps> = ({
  ids,
  startMsOf,
  stale,
  onJump,
  slideIds = [],
  slideStartMsOf,
  onJumpSlide,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  // Eine Nummer ohne Segment (geloescht, anderes Transkript) hat keine Zeit
  // und damit kein Ziel; im veralteten Zustand zeigen alle nur einen Platzhalter.
  const usable = stale ? ids : ids.filter((id) => startMsOf(id) !== null);
  // Folien haengen nicht an der Epoche des Transkripts: sie zaehlen, solange es sie gibt.
  const slides = slideStartMsOf
    ? slideIds.filter((n) => slideStartMsOf(n) !== null)
    : [];
  if (usable.length === 0 && slides.length === 0) return null;
  const { shown, rest } = expanded
    ? { shown: usable, rest: [] as number[] }
    : splitSources(usable, MAX_VISIBLE_SOURCES);

  const chipClass =
    "inline-flex items-center gap-0.5 rounded-md border border-mid-gray/30 px-1.5 py-0.5 text-[11px] leading-none tabular-nums";

  return (
    <span
      data-testid="source-chips"
      className="inline-flex flex-wrap items-center justify-end gap-1"
    >
      {slides.map((number) => {
        const start = slideStartMsOf?.(number) ?? null;
        const time = start === null ? "--:--" : formatAt(start);
        const label = t("meetings.enhanced.slideJump", { number, time });
        return (
          <button
            key={`slide-${number}`}
            type="button"
            data-testid="source-slide"
            data-slide-number={number}
            onClick={() => onJumpSlide?.(number)}
            title={label}
            aria-label={label}
            className={`${chipClass} cursor-pointer text-text/60 hover:border-logo-primary hover:text-text focus:outline-none focus-visible:border-logo-primary`}
          >
            <Presentation width={10} height={10} aria-hidden="true" />
            {`F${number}`}
          </button>
        );
      })}
      {shown.map((id) => {
        const start = startMsOf(id);
        const label = stale || start === null ? "--:--" : formatAt(start);
        return (
          <button
            key={id}
            type="button"
            data-source-id={id}
            disabled={stale}
            onClick={() => onJump(id)}
            title={
              stale
                ? t("meetings.enhanced.sourceStale")
                : t("meetings.enhanced.sourceJump", { time: label })
            }
            aria-label={
              stale
                ? t("meetings.enhanced.sourceStale")
                : t("meetings.enhanced.sourceJump", { time: label })
            }
            className={`${chipClass} text-text/60 ${
              stale
                ? "cursor-not-allowed opacity-50"
                : "cursor-pointer hover:border-logo-primary hover:text-text focus:outline-none focus-visible:border-logo-primary"
            }`}
          >
            <Search width={10} height={10} aria-hidden="true" />
            {label}
          </button>
        );
      })}
      {rest.length > 0 && (
        <button
          type="button"
          data-testid="source-more"
          aria-expanded={expanded}
          onClick={() => setExpanded(true)}
          title={t("meetings.enhanced.sourceMore", { count: rest.length })}
          className={`${chipClass} cursor-pointer text-text/50 hover:border-logo-primary hover:text-text focus:outline-none focus-visible:border-logo-primary`}
        >
          +{rest.length}
        </button>
      )}
    </span>
  );
};
