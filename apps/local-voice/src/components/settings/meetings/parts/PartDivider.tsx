import React from "react";
import { useTranslation } from "react-i18next";
import { Layers } from "lucide-react";
import type { MeetingPart, StoredSegment } from "@/bindings";
import { formatPartTime } from "./useMeetingParts";

/**
 * Wo im Transkript ein Teil beginnt: das erste Segment ab `offset_ms` jedes
 * Teils ausser dem ersten (Teil 1 beginnt bei 0:00 und braucht keine Marke).
 */
export const partStartsBySegment = (
  segments: StoredSegment[],
  parts: MeetingPart[],
): Map<number, MeetingPart> => {
  const out = new Map<number, MeetingPart>();
  if (parts.length < 2) return out;
  const sorted = [...segments].sort((a, b) => a.start_ms - b.start_ms);
  parts.forEach((part, i) => {
    if (i === 0) return;
    // Nur Segmente innerhalb des Teils: ein Teil ohne Sprache bekommt keine
    // Marke und nimmt dem naechsten seine nicht weg.
    const end = parts[i + 1]?.offset_ms ?? Number.POSITIVE_INFINITY;
    const first = sorted.find(
      (s) => s.start_ms >= part.offset_ms && s.start_ms < end,
    );
    if (first) out.set(first.segment_index, part);
  });
  return out;
};

/**
 * Trennzeile im Transkript am Beginn eines Teils ("Teil 2 · Aufnahme 113.m4a ·
 * ab 1:13:52"). Ein Klick spielt ab dort; unter 640 px mindestens 44 px hoch.
 */
export const PartDivider: React.FC<{
  part: MeetingPart;
  onSeek?: (ms: number) => void;
}> = ({ part, onSeek }) => {
  const { t } = useTranslation();
  const label = t("meetings.parts.divider", {
    n: part.part_index + 1,
    title: part.title,
    at: formatPartTime(part.offset_ms),
  });
  return (
    <div
      className="flex items-center gap-2 py-1 text-xs text-text/60"
      data-testid="part-divider"
    >
      <span className="h-px flex-1 bg-mid-gray/30" aria-hidden="true" />
      <button
        type="button"
        disabled={!onSeek}
        onClick={() => onSeek?.(part.offset_ms)}
        title={`${label}\n${t("meetings.parts.playFrom")}`}
        aria-label={label}
        className="inline-flex max-w-[80%] items-center gap-1 rounded-full border max-sm:rounded-lg max-sm:text-start border-mid-gray/30 px-2 py-0.5 hover:bg-mid-gray/15 disabled:cursor-default max-sm:min-h-[44px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
        data-testid="part-divider-button"
      >
        <Layers
          width={12}
          height={12}
          aria-hidden="true"
          className="shrink-0"
        />
        <span className="truncate max-sm:whitespace-normal max-sm:[overflow-wrap:anywhere]">
          {label}
        </span>
      </button>
      <span className="h-px flex-1 bg-mid-gray/30" aria-hidden="true" />
    </div>
  );
};
