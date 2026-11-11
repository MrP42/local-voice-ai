import React, { useContext, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ChevronDown, Sparkles } from "lucide-react";
import { commands, type Prep } from "@/bindings";
import { requestOpenSegment } from "@/lib/meetingsBus";
import { useSettings } from "../../../../hooks/useSettings";
import { PrepSlotContext } from "./prepSlot";

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
 *
 * Beim bloßen Anzeigen entstehen die Punkte aus den Regeln oder mit einem lokalen
 * Modell. Läge ein Cloud-Modell bereit, bietet die Karte "Mit KI zusammenfassen"
 * an: erst der Klick sendet etwas (das Backend erzwingt das über den Auslöser).
 *
 * Kein Layoutsprung: Im Arbeitsbereich schwebt die Karte über dem unteren Rand der
 * Notizfläche (`PrepSlotContext`), statt den Fluss zu verschieben; Eintreffen und
 * Einklappen bewegen nichts, was darüber steht. Das Erscheinen wird höflich
 * angesagt (`aria-live`).
 */
export const PrepCard: React.FC<PrepCardProps> = ({ eventKey }) => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const slot = useContext(PrepSlotContext);
  const enabled = getSetting("meeting_prep_enabled") !== false;
  const [prep, setPrep] = useState<Prep | null>(null);
  const [expanded, setExpanded] = useState(true);
  const [summarizing, setSummarizing] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const cardRef = useRef<HTMLElement | null>(null);
  const keyRef = useRef(eventKey);
  const bodyId = useId();

  useEffect(() => {
    keyRef.current = eventKey;
    setPrep(null);
    setExpanded(true);
    setSummarizing(false);
    setUnavailable(false);
    if (!enabled) return;
    let cancelled = false;
    void commands.meetingPrepGet(eventKey, "view").then((result) => {
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

  const summarize = () => {
    const key = eventKey;
    setSummarizing(true);
    setUnavailable(false);
    void commands.meetingPrepGet(key, "click").then((result) => {
      if (keyRef.current !== key) return;
      setSummarizing(false);
      if (result.status !== "ok") {
        setUnavailable(true);
        return;
      }
      setPrep(result.data);
      // Das Modell war nicht erreichbar oder lieferte nichts Brauchbares.
      setUnavailable(result.data.mode !== "model");
    });
  };

  const dateOf = (seconds: number) =>
    new Date(seconds * 1000).toLocaleDateString(i18n.language, {
      day: "2-digit",
      month: "2-digit",
      year: "numeric",
    });

  const card =
    enabled && prep !== null && prep.points.length > 0 ? (
      <section
        ref={cardRef}
        aria-label={t("meetings.prep.title")}
        className="pointer-events-auto rounded-lg border border-mid-gray/30 bg-background text-sm shadow-lg"
        data-testid="prep-card"
        data-mode={prep.mode}
        data-expanded={expanded ? "true" : "false"}
      >
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          aria-expanded={expanded}
          aria-controls={bodyId}
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
        <div
          id={bodyId}
          hidden={!expanded}
          className="max-h-[40vh] space-y-2 overflow-y-auto px-3 pb-3"
        >
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
          {prep.summarizable && (
            <div className="space-y-1" data-testid="prep-summarize-area">
              <button
                type="button"
                onClick={summarize}
                disabled={summarizing}
                className="inline-flex cursor-pointer items-center gap-1.5 rounded-md border border-mid-gray/30 px-2.5 py-1 text-xs font-medium hover:bg-mid-gray/10 disabled:cursor-default disabled:opacity-60"
                data-testid="prep-summarize"
              >
                <Sparkles width={12} height={12} aria-hidden="true" />
                {summarizing
                  ? t("meetings.prep.summarizing")
                  : t("meetings.prep.summarize")}
              </button>
              <p className="text-xs text-text/50">
                {t("meetings.prep.summarizeHint")}
              </p>
            </div>
          )}
          {unavailable && (
            <p className="text-xs text-text/60" data-testid="prep-unavailable">
              {t("meetings.prep.summarizeUnavailable")}
            </p>
          )}
        </div>
      </section>
    ) : null;

  if (slot) return createPortal(card, slot);
  return <div aria-live="polite">{card}</div>;
};
