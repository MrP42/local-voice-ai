import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
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
}

/**
 * Quellen-Chips eines KI-Eintrags: die Zeit (`mm:ss`) der belegenden Stelle,
 * hoechstens drei, der Rest hinter "+n". Ein Klick springt ins Transkript.
 */
export const SourceChips: React.FC<SourceChipsProps> = ({
  ids,
  startMsOf,
  stale,
  onJump,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  // Eine Nummer ohne Segment (geloescht, anderes Transkript) hat keine Zeit
  // und damit kein Ziel; im veralteten Zustand zeigen alle nur einen Platzhalter.
  const usable = stale ? ids : ids.filter((id) => startMsOf(id) !== null);
  if (usable.length === 0) return null;
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
