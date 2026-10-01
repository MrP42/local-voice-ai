import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CalendarClock, ChevronDown, ChevronRight, Mic } from "lucide-react";
import { commands, type BriefInfo, type CalEvent } from "@/bindings";
import { useRecordingActive } from "../../../../hooks/useRecordingActive";
import { distinctAttendees, todaysEvents } from "@/lib/meetingCalendar";
import { requestRecordingStart } from "@/lib/recordingStartRequest";
import { IconAction } from "../../../ui/IconAction";
import { BriefButton } from "../people/BriefButton";

const REFRESH_MS = 60_000;
/** Wie weit voraus der naechste Termin gesucht wird (7 Tage). */
const LOOKAHEAD_HOURS = 24 * 7;

/** Der naechste Termin: nicht ganztaegig, nicht abgesagt, noch nicht zu Ende. */
export const nextEvent = (events: CalEvent[], nowMs: number): CalEvent | null =>
  events
    .filter((e) => !e.all_day && !e.cancelled && e.ends_at > nowMs)
    .sort(
      (a, b) => a.starts_at - b.starts_at || a.key.localeCompare(b.key),
    )[0] ?? null;

const startOfDay = (ms: number) => {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};

interface NextUpProps {
  /** M5-P5e: "Vorbereiten" an einem Termin (oeffnet den Brief im Chat). */
  onPrepare?: (info: BriefInfo) => void;
}

/**
 * "Als Naechstes": der naechste Kalendertermin (P5b) als kleiner fester
 * Abschnitt am Fuss der Projekte-Spalte, mit einem Knopf, der den Startdialog
 * zu diesem Termin oeffnet (Titel, Projekt, Vorlage, Einwilligung - der Dialog
 * gehoert der Aufnahmekarte, hier geht nur der Wunsch hin). Die uebrigen Termine
 * des Tages stehen darunter unter "Weitere Termine heute", jeweils mit
 * "Vorbereiten" und Aufnehmen. Ohne Kalenderquelle erscheint nichts.
 */
export const NextUp: React.FC<NextUpProps> = ({ onPrepare }) => {
  const { t, i18n } = useTranslation();
  const recording = useRecordingActive();
  const [hasCalendar, setHasCalendar] = useState(false);
  const [event, setEvent] = useState<CalEvent | null>(null);
  const [others, setOthers] = useState<CalEvent[]>([]);
  const [othersOpen, setOthersOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const refresh = async () => {
      const [sources, list] = await Promise.all([
        commands.calendarSourcesList(),
        commands.calendarUpcoming(LOOKAHEAD_HOURS),
      ]);
      if (cancelled) return;
      setHasCalendar(
        sources.status === "ok" && (sources.data ?? []).length > 0,
      );
      const now = Date.now();
      const events = list.status === "ok" ? (list.data ?? []) : [];
      const next = nextEvent(events, now);
      setEvent(next);
      setOthers(todaysEvents(events, now).filter((e) => e.key !== next?.key));
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  if (!hasCalendar && !event) return null;

  const timeOf = (ms: number) =>
    new Date(ms).toLocaleTimeString(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
    });

  const whenLabel = (e: CalEvent) => {
    const now = Date.now();
    const time = timeOf(e.starts_at);
    if (e.starts_at <= now) return t("meetings.projects.nextUp.running");
    const days = Math.round(
      (startOfDay(e.starts_at) - startOfDay(now)) / 86_400_000,
    );
    if (days <= 0) return t("meetings.projects.nextUp.today", { time });
    if (days === 1) return t("meetings.projects.nextUp.tomorrow", { time });
    const day = new Date(e.starts_at).toLocaleDateString(i18n.language, {
      weekday: "short",
      day: "numeric",
      month: "short",
    });
    return `${day}, ${time}`;
  };

  const attendeesOf = (e: CalEvent) => {
    const count = distinctAttendees(e);
    return count > 0
      ? ` · ${t("meetings.calendar.upcoming.attendees", { count })}`
      : "";
  };

  const startButton = (e: CalEvent) => (
    <IconAction
      size="sm"
      icon={Mic}
      label={t("meetings.projects.nextUp.start")}
      description={t("meetings.projects.nextUp.startHint")}
      testId="next-up-start"
      disabled={recording.active}
      onClick={() => requestRecordingStart(e)}
    />
  );

  return (
    <section
      aria-label={t("meetings.projects.nextUp.title")}
      data-testid="next-up"
      className="shrink-0 border-t border-mid-gray/20 px-1 pt-2"
    >
      <h3 className="px-1 pb-1 text-xs font-semibold uppercase tracking-wide text-text/60">
        {t("meetings.projects.nextUp.title")}
      </h3>
      {event ? (
        <div className="space-y-1 px-1">
          <div className="flex items-center gap-2">
            <CalendarClock
              width={16}
              height={16}
              aria-hidden="true"
              className="shrink-0 text-text/60"
            />
            <div className="min-w-0 flex-1 text-sm">
              <p className="truncate font-medium" data-testid="next-up-title">
                {event.title}
              </p>
              <p className="truncate text-xs text-text/60">
                {whenLabel(event)}
                {attendeesOf(event)}
              </p>
            </div>
            {startButton(event)}
          </div>
          {onPrepare && (
            <BriefButton
              eventKey={event.key}
              onOpen={onPrepare}
              testId="upcoming-brief"
            />
          )}
        </div>
      ) : (
        <p className="px-1 text-sm text-text/60">
          {t("meetings.projects.nextUp.empty")}
        </p>
      )}

      {others.length > 0 && (
        <div className="mt-1" data-testid="upcoming-card">
          <button
            type="button"
            onClick={() => setOthersOpen((v) => !v)}
            aria-expanded={othersOpen}
            className="flex w-full cursor-pointer items-center gap-1 rounded-md px-1 py-1 text-xs font-medium text-text/70 hover:text-text"
            data-testid="upcoming-toggle"
          >
            {othersOpen ? (
              <ChevronDown width={14} height={14} aria-hidden="true" />
            ) : (
              <ChevronRight width={14} height={14} aria-hidden="true" />
            )}
            {t("meetings.projects.nextUp.more", { count: others.length })}
          </button>
          {othersOpen && (
            <ul className="max-h-40 space-y-2 overflow-y-auto px-1 pb-1">
              {others.map((other) => (
                <li
                  key={other.key}
                  className="space-y-1 text-sm"
                  data-testid="upcoming-event"
                >
                  <div className="flex items-center gap-2">
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-medium">{other.title}</p>
                      <p className="truncate text-xs text-text/60">
                        <span className="tabular-nums">
                          {timeOf(other.starts_at)}–{timeOf(other.ends_at)}
                        </span>
                        {attendeesOf(other)}
                      </p>
                    </div>
                    <IconAction
                      size="sm"
                      icon={Mic}
                      label={t("meetings.projects.nextUp.start")}
                      description={t("meetings.projects.nextUp.startHint")}
                      testId="upcoming-start"
                      disabled={recording.active}
                      onClick={() => requestRecordingStart(other)}
                    />
                  </div>
                  {onPrepare && (
                    <BriefButton
                      eventKey={other.key}
                      onOpen={onPrepare}
                      testId="upcoming-brief"
                    />
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
};
