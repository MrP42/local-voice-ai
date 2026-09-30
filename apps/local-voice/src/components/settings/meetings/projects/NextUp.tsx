import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CalendarClock, Mic } from "lucide-react";
import { toast } from "sonner";
import { commands, type CalEvent } from "@/bindings";
import { useSettings } from "../../../../hooks/useSettings";
import { useRecordingActive } from "../../../../hooks/useRecordingActive";
import { distinctAttendees } from "@/lib/meetingCalendar";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { IconAction } from "../../../ui/IconAction";
import { translateMeetingError } from "../meetingErrors";
import { MeetingChatNotice } from "../MeetingChatNotice";

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

/**
 * "Als Naechstes": der naechste Kalendertermin (P5b) als kleiner fester
 * Abschnitt am Fuss der Projekte-Spalte, mit einem Knopf, der die Aufnahme
 * dazu startet (nach der Einwilligung, wie auf der Aufnahmekarte). Ohne
 * Kalenderquelle erscheint nichts.
 */
export const NextUp: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const recording = useRecordingActive();
  const [hasCalendar, setHasCalendar] = useState(false);
  const [event, setEvent] = useState<CalEvent | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [busy, setBusy] = useState(false);

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
      setEvent(
        list.status === "ok" ? nextEvent(list.data ?? [], Date.now()) : null,
      );
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  if (!hasCalendar && !event) return null;

  const whenLabel = (e: CalEvent) => {
    const now = Date.now();
    const time = new Date(e.starts_at).toLocaleTimeString(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
    });
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

  const confirmStart = async () => {
    if (!event) return;
    setBusy(true);
    const capture = getSetting("meeting_capture_system") ?? true;
    const result = await commands.meetingsStartFromEvent(
      event.key,
      null,
      true,
      capture,
      null,
      "prompt",
    );
    setBusy(false);
    setConsentOpen(false);
    if (result.status === "error") {
      toast.error(translateMeetingError(result.error, t));
    }
  };

  const attendees = event ? distinctAttendees(event) : 0;

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
        <div className="flex items-center gap-2 px-1">
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
              {attendees > 0 &&
                ` · ${t("meetings.calendar.upcoming.attendees", { count: attendees })}`}
            </p>
          </div>
          <IconAction
            size="sm"
            icon={Mic}
            label={t("meetings.projects.nextUp.start")}
            description={t("meetings.projects.nextUp.startHint")}
            testId="next-up-start"
            disabled={recording.active || busy}
            onClick={() => setConsentOpen(true)}
          />
        </div>
      ) : (
        <p className="px-1 text-sm text-text/60">
          {t("meetings.projects.nextUp.empty")}
        </p>
      )}

      <Dialog
        open={consentOpen}
        onOpenChange={setConsentOpen}
        title={t("meetings.consent.title")}
        closeLabel={t("meetings.consent.cancel")}
        footer={
          <>
            <Button
              variant="secondary"
              onClick={() => setConsentOpen(false)}
              disabled={busy}
            >
              {t("meetings.consent.cancel")}
            </Button>
            <Button onClick={confirmStart} disabled={busy}>
              {t("meetings.consent.confirm")}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <p className="text-sm text-text/80 whitespace-pre-wrap">
            {t("meetings.consent.body")}
          </p>
          <MeetingChatNotice testId="next-up-consent" />
        </div>
      </Dialog>
    </section>
  );
};
