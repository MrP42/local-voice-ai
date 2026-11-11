import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Sparkles } from "lucide-react";
import { commands, type Prep } from "@/bindings";
import { requestOpenSegment } from "@/lib/meetingsBus";
import { useSettings } from "../../../../hooks/useSettings";

interface PrepCardProps {
  /** Schlüssel des Termins (`CalEvent.key`). */
  eventKey: string;
}

/** Schreibt der Nutzer gerade in ein Textfeld außerhalb der Karte (Notizen)? */
const isTypingElsewhere = (event: Event, card: HTMLElement | null): boolean => {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;
  if (card?.contains(target)) return false;
  return (
    target instanceof HTMLTextAreaElement ||
    target.isContentEditable ||
    target.closest("[contenteditable='true']") !== null
  );
};

/**
 * G4 (#499): einklappbare Karte "Vorbereitet aus deinen letzten Notizen" am
 * Termin: zwei bis drei offene Punkte aus den eigenen früheren Notizen, jeder mit
 * Quelle (Besprechung und Datum, ein Klick springt dorthin). Ohne passende
 * frühere Notiz (oder bei ausgeschalteter Funktion) erscheint nichts, kein
 * Fülltext. Sobald der Nutzer in die Notizen schreibt, klappt die Karte ein.
 */
export const PrepCard: React.FC<PrepCardProps> = ({ eventKey }) => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const enabled = getSetting("meeting_prep_enabled") !== false;
  const [prep, setPrep] = useState<Prep | null>(null);
  const [expanded, setExpanded] = useState(true);
  const cardRef = React.useRef<HTMLElement | null>(null);

  useEffect(() => {
    setPrep(null);
    setExpanded(true);
    if (!enabled) return;
    let cancelled = false;
    void commands.meetingPrepGet(eventKey).then((result) => {
      if (cancelled) return;
      if (result.status === "ok") setPrep(result.data);
    });
    return () => {
      cancelled = true;
    };
  }, [eventKey, enabled]);

  const visible = prep !== null && prep.points.length > 0;
  useEffect(() => {
    if (!visible) return;
    const onInput = (event: Event) => {
      if (isTypingElsewhere(event, cardRef.current)) setExpanded(false);
    };
    document.addEventListener("input", onInput, true);
    return () => document.removeEventListener("input", onInput, true);
  }, [visible]);

  if (!enabled || prep === null || prep.points.length === 0) return null;

  const dateOf = (seconds: number) =>
    new Date(seconds * 1000).toLocaleDateString(i18n.language);

  return (
    <section
      ref={cardRef}
      aria-label={t("meetings.prep.title")}
      className="rounded-lg border border-mid-gray/30 bg-mid-gray/5 text-sm"
      data-testid="prep-card"
      data-mode={prep.mode}
      data-expanded={expanded ? "true" : "false"}
    >
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        title={
          expanded ? t("meetings.prep.collapse") : t("meetings.prep.expand")
        }
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left font-medium"
        data-testid="prep-toggle"
      >
        <Sparkles width={14} height={14} aria-hidden="true" />
        <span className="flex-1">{t("meetings.prep.title")}</span>
        <ChevronDown
          width={14}
          height={14}
          aria-hidden="true"
          className={expanded ? "rotate-180" : ""}
        />
      </button>
      {expanded && (
        <div className="space-y-2 px-3 pb-3">
          <ul className="space-y-2">
            {prep.points.map((point, index) => {
              const date = dateOf(point.source.at);
              return (
                <li
                  key={`${point.source.meeting_id}-${index}`}
                  data-testid="prep-point"
                  data-kind={point.kind}
                >
                  <div>{point.text}</div>
                  <button
                    type="button"
                    onClick={() =>
                      requestOpenSegment({
                        meetingId: point.source.meeting_id,
                        segmentIndex: null,
                      })
                    }
                    aria-label={t("meetings.prep.openSource", {
                      title: point.source.title,
                      date,
                    })}
                    className="cursor-pointer text-xs text-text/60 underline hover:text-text"
                    data-testid="prep-source"
                  >
                    {t("meetings.prep.source", {
                      title: point.source.title,
                      date,
                    })}
                  </button>
                </li>
              );
            })}
          </ul>
          <p className="text-xs text-text/50" data-testid="prep-mode">
            {prep.mode === "model"
              ? t("meetings.prep.modeModel")
              : t("meetings.prep.modeRules")}
          </p>
        </div>
      )}
    </section>
  );
};
