import React, { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Citation } from "@/bindings";
import { Tooltip } from "../../../ui/Tooltip";
import { formatAt } from "@/lib/meetingNotes";
import { formatDay } from "@/lib/meetingChat";

interface CitationChipProps {
  n: number;
  /** `undefined`: Nummer ohne Zitat (sollte nicht vorkommen) - nur Text. */
  citation: Citation | undefined;
  onJump?: (citation: Citation) => void;
}

/**
 * Ein Beleg `[n]` als kleiner Chip im Antworttext. Tooltip: Besprechung,
 * Datum, Stelle (`mm:ss` bzw. Notizen) und das Zitat; ein Klick springt
 * dorthin (Transkript: Segment markieren und abspielen; Notizen: Tab Notizen).
 */
export const CitationChip: React.FC<CitationChipProps> = ({
  n,
  citation,
  onJump,
}) => {
  const { t, i18n } = useTranslation();
  const ref = useRef<HTMLButtonElement>(null);
  const [hover, setHover] = useState(false);
  const tipId = useId();
  // Der Antworttext, in dem der Chip steht: die Sprechblase darf ihn nicht
  // ueberdecken (B6) und weicht daneben bzw. darunter aus.
  const answerRef = useRef<HTMLElement | null>(null);
  const show = () => {
    answerRef.current =
      ref.current?.closest<HTMLElement>("[data-citation-scope]") ?? null;
    setHover(true);
  };

  if (!citation) return <span>[{n}]</span>;

  // D5: ein Folien-Beleg nennt die Quelle und die Zeit der Folie ("Folie 04:12").
  const where =
    citation.source === "transcript" && citation.start_ms !== null
      ? formatAt(citation.start_ms)
      : citation.source === "slide" && citation.start_ms !== null
        ? `${t("meetings.chat.citation.source.slide")} ${formatAt(citation.start_ms)}`
        : t(`meetings.chat.citation.source.${citation.source}`);
  const day = formatDay(citation.started_at, i18n.language);
  const label = `${t("meetings.chat.citation.label", { n })}: ${citation.meeting_title}, ${where}`;

  return (
    <>
      <button
        ref={ref}
        type="button"
        data-testid="citation-chip"
        data-cite={n}
        aria-label={label}
        aria-describedby={hover ? tipId : undefined}
        onMouseEnter={show}
        onMouseLeave={() => setHover(false)}
        onFocus={show}
        onBlur={() => setHover(false)}
        onClick={() => onJump?.(citation)}
        className="mx-0.5 inline-flex min-w-[1.25rem] items-center justify-center rounded-md border border-logo-primary/50 bg-logo-primary/10 px-1 align-baseline text-[11px] font-medium leading-4 tabular-nums text-text hover:bg-logo-primary/25 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60 cursor-pointer"
      >
        {n}
      </button>
      {hover && (
        <Tooltip targetRef={ref} position="top" avoidRef={answerRef}>
          <div role="tooltip" id={tipId} className="space-y-1 text-xs">
            <p className="font-medium text-text">{citation.meeting_title}</p>
            <p className="text-text/60 tabular-nums">
              {[day, where].filter(Boolean).join(" · ")}
            </p>
            {citation.quote && (
              <p className="italic text-text/80">„{citation.quote}“</p>
            )}
          </div>
        </Tooltip>
      )}
    </>
  );
};
