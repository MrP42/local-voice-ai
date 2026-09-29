import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, X } from "lucide-react";
import {
  commands,
  events,
  type BriefInfo,
  type CalEvent,
  type HealthState,
} from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { Dialog } from "../../ui/Dialog";
import { Alert } from "../../ui/Alert";
import Badge from "../../ui/Badge";
import { translateMeetingError } from "./meetingErrors";
import { MeetingChatNotice } from "./MeetingChatNotice";
import { TemplatePicker } from "./notes/TemplatePicker";
import { BriefButton } from "./people/BriefButton";
import { flushAllNotes } from "./notes/useNotesAutosave";
import {
  distinctAttendees,
  hoursUntilEndOfDay,
  todaysEvents,
} from "@/lib/meetingCalendar";

type Phase = "idle" | "recording" | "paused";

/** Stand der KI-Notizen, die nach dem Stopp automatisch entstehen (M1-P1f). */
type AutoNotes =
  | { kind: "running"; step: number; total: number }
  | { kind: "done" }
  | { kind: "failed"; code: string };

/** Codes von `MeetingNotesEvent::Failed`, fuer die es einen eigenen Text gibt. */
const AUTO_NOTES_CODES = new Set([
  "no_provider",
  "no_model",
  "memory_low",
  "recording_active",
  "enhance_busy",
  "no_transcript",
  "llm_failed",
]);

/**
 * Warnungen des Ausfallwaechters (M2-P2e). Je Kanal steht hoechstens EINE
 * Bedingung (`conditions`); `recovered` nimmt sie zurueck. `vad_unavailable`
 * und `loopback_died` bleiben stehen, bis die Aufnahme endet.
 */
type Condition = "no_data" | "digital_zero" | "silent" | "clipping";
type HealthView = {
  conditions: Partial<Record<number, Condition | "queue_overflow">>;
  sticky: Partial<Record<number, "vad_unavailable" | "loopback_died">>;
};
const NO_HEALTH: HealthView = { conditions: {}, sticky: {} };

const applyHealth = (
  view: HealthView,
  channel: number,
  state: HealthState,
): HealthView => {
  if (state === "recovered") {
    if (view.conditions[channel] === undefined) return view;
    const conditions = { ...view.conditions };
    delete conditions[channel];
    return { ...view, conditions };
  }
  if (state === "vad_unavailable" || state === "loopback_died") {
    return { ...view, sticky: { ...view.sticky, [channel]: state } };
  }
  return { ...view, conditions: { ...view.conditions, [channel]: state } };
};

const LevelBar: React.FC<{ label: string; value: number }> = ({
  label,
  value,
}) => (
  <div className="flex items-center gap-2">
    <span className="text-xs text-text/60 w-20 shrink-0">{label}</span>
    <div className="h-1.5 flex-1 rounded-full bg-mid-gray/20 overflow-hidden">
      <div
        className="h-full rounded-full bg-logo-primary"
        style={{
          width: `${Math.min(100, Math.max(0, value * 100))}%`,
          transition: "width 80ms linear",
        }}
      />
    </div>
  </div>
);

/** Termin, dem die naechste Aufnahme gehoert (M5-P5b). `auto` = Titelvorschlag, `prompt` = Terminkarte. */
type EventChoice = { event: CalEvent; mode: "auto" | "prompt" };

const CALENDAR_REFRESH_MS = 60_000;

interface RecorderCardProps {
  /** M5-P5e: "Vorbereiten" an einem Termin der Karte "Naechste Termine". */
  onPrepare?: (info: BriefInfo) => void;
}

export const RecorderCard: React.FC<RecorderCardProps> = ({ onPrepare }) => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const [title, setTitle] = useState("");
  // Vorgabe und letzte Wahl liegen in den Einstellungen (`meeting_capture_system`,
  // Standard an): das Haekchen ist kein eigener UI-Zustand mehr.
  const captureSetting = getSetting("meeting_capture_system") ?? true;
  // Womit die laufende Aufnahme gestartet wurde; ohne Wert (Seite mitten in
  // einer Aufnahme geoeffnet) gilt die Einstellung.
  const [startedWithSystem, setStartedWithSystem] = useState<boolean | null>(
    null,
  );
  // M3-P3c: mehrere Personen am Mikrofon (Raum): je Besprechung, nicht gemerkt.
  const [diarizeMic, setDiarizeMic] = useState(false);
  // Vorlage fuer die naechste Besprechung: bis der Nutzer waehlt, gilt die
  // Standardvorlage aus den Einstellungen (`null` = Standardvorlage).
  const defaultTemplate = getSetting("meeting_default_template_id") ?? null;
  const [templateChoice, setTemplateChoice] = useState<string | null>(null);
  const templateId = templateChoice ?? defaultTemplate;
  const [phase, setPhase] = useState<Phase>("idle");
  const [consentOpen, setConsentOpen] = useState(false);
  const [micLevel, setMicLevel] = useState(0);
  const [systemLevel, setSystemLevel] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [autoNotes, setAutoNotes] = useState<AutoNotes | null>(null);
  const [health, setHealth] = useState<HealthView>(NO_HEALTH);
  // Kalender (M5-P5b): Vorschlag aus laufendem/naechstem Termin (+-15 min),
  // gewaehlter Termin und die Liste "Naechste Termine (heute)".
  const [suggestion, setSuggestion] = useState<CalEvent | null>(null);
  const [eventChoice, setEventChoice] = useState<EventChoice | null>(null);
  const [suggestionCleared, setSuggestionCleared] = useState(false);
  const [upcoming, setUpcoming] = useState<CalEvent[]>([]);
  const [hasCalendar, setHasCalendar] = useState(false);
  const [upcomingOpen, setUpcomingOpen] = useState(true);
  // Termine, zu denen schon aufgenommen wurde: kein zweiter Vorschlag.
  const usedEventKeys = useRef(new Set<string>());
  // Besprechung, deren automatische KI-Notizen die Statuszeile zeigt.
  const notesMeetingRef = useRef<string | null>(null);

  useEffect(() => {
    commands.meetingsIsRecording().then((r) => {
      if (r.status === "ok" && r.data) setPhase("recording");
    });

    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind === "state") {
        if (payload.status === "recording" || payload.status === "paused") {
          setPhase(payload.paused ? "paused" : "recording");
        } else {
          setPhase("idle");
          if (payload.status === "processing") {
            // Ende der Aufnahme: ab jetzt gehoert die Statuszeile dieser Besprechung.
            notesMeetingRef.current = payload.meeting_id;
            setAutoNotes(null);
          }
        }
      } else if (payload.kind === "levels") {
        setMicLevel(payload.mic);
        setSystemLevel(payload.system);
      } else if (payload.kind === "error") {
        setError(translateMeetingError(payload.message, t));
        setBusy(false);
      } else if (payload.kind === "health") {
        setHealth((v) => applyHealth(v, payload.channel, payload.state));
      }
    });
    const unNotes = events.meetingNotesEvent.listen((e) => {
      const payload = e.payload;
      if (payload.meeting_id !== notesMeetingRef.current) return;
      if (payload.kind === "progress") {
        setAutoNotes({
          kind: "running",
          step: payload.step,
          total: payload.total,
        });
      } else if (payload.kind === "done") {
        setAutoNotes({ kind: "done" });
      } else {
        setAutoNotes({ kind: "failed", code: payload.code });
      }
    });
    return () => {
      un.then((f) => f());
      unNotes.then((f) => f());
    };
  }, []);

  const idle = phase === "idle";
  useEffect(() => {
    if (!idle) return;
    let cancelled = false;
    const refresh = async () => {
      const now = Date.now();
      const [sources, near, list] = await Promise.all([
        commands.calendarSourcesList(),
        commands.calendarSuggestEvent(),
        commands.calendarUpcoming(hoursUntilEndOfDay(now)),
      ]);
      if (cancelled) return;
      if (sources.status === "ok")
        setHasCalendar((sources.data ?? []).length > 0);
      if (near.status === "ok") setSuggestion(near.data ?? null);
      if (list.status === "ok") setUpcoming(todaysEvents(list.data ?? [], now));
    };
    void refresh();
    const timer = setInterval(() => void refresh(), CALENDAR_REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [idle]);

  // Titelvorschlag: nur solange der Nutzer nichts eingegeben und den Bezug
  // nicht selbst geloest hat.
  useEffect(() => {
    if (!suggestion || suggestionCleared || eventChoice || title !== "") return;
    if (usedEventKeys.current.has(suggestion.key)) return;
    setEventChoice({ event: suggestion, mode: "auto" });
    setTitle(suggestion.title);
  }, [suggestion, suggestionCleared, eventChoice, title]);

  const openConsent = () => {
    setError(null);
    setConsentOpen(true);
  };

  const confirmStart = async () => {
    setBusy(true);
    setError(null);
    const typed = title.trim();
    const result = eventChoice
      ? await commands.meetingsStartFromEvent(
          eventChoice.event.key,
          null,
          true,
          captureSetting,
          // Nur ein anderer Titel als der des Termins ist eine Eingabe.
          typed && typed !== eventChoice.event.title ? typed : null,
          eventChoice.mode,
        )
      : await commands.meetingsStart(
          typed || t("meetings.record.titlePlaceholder"),
          true,
          captureSetting,
        );
    setBusy(false);
    setConsentOpen(false);
    if (result.status === "error") {
      setError(translateMeetingError(result.error, t));
      return;
    }
    setStartedWithSystem(captureSetting);
    if (diarizeMic && captureSetting) {
      void commands.meetingsSetDiarizeMic(result.data.id, true);
    }
    setDiarizeMic(false);
    setAutoNotes(null);
    setHealth(NO_HEALTH);
    notesMeetingRef.current = result.data.id;
    setPhase("recording");
    // Gewaehlte Vorlage der Besprechung zuordnen; ohne Wahl gilt die
    // Standardvorlage (der Motor faellt bei `None` selbst darauf zurueck).
    // Bei einem Termin gilt sonst die Vorlage der Serie (Backend); die
    // Standardvorlage darf sie nicht ueberschreiben, nur eine Wahl des Nutzers.
    const effectiveTemplate = eventChoice ? templateChoice : templateId;
    if (effectiveTemplate) {
      void commands.meetingsSetTemplate(result.data.id, effectiveTemplate);
    }
    if (eventChoice) usedEventKeys.current.add(eventChoice.event.key);
    setEventChoice(null);
    setSuggestionCleared(false);
  };

  const startFromCard = (event: CalEvent) => {
    setEventChoice({ event, mode: "prompt" });
    setTitle(event.title);
    openConsent();
  };

  const clearEventChoice = () => {
    setEventChoice(null);
    setSuggestionCleared(true);
  };

  const timeOf = (ms: number) =>
    new Date(ms).toLocaleTimeString(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
    });

  const pause = () => {
    void commands.meetingsPause();
  };

  const resume = () => {
    void commands.meetingsResume();
  };

  const stop = async () => {
    setBusy(true);
    // Ungespeicherte Notizen sichern, bevor die Aufnahme endet: danach laeuft
    // der Auto-Lauf und liest den Notizblock.
    await flushAllNotes();
    const result = await commands.meetingsStop();
    setBusy(false);
    if (result.status === "error") {
      setError(translateMeetingError(result.error, t));
      return;
    }
    setPhase("idle");
    setTitle("");
    setEventChoice(null);
    setSuggestionCleared(false);
    setMicLevel(0);
    setSystemLevel(0);
    setStartedWithSystem(null);
    setHealth(NO_HEALTH);
  };

  const recording = phase === "recording";
  const paused = phase === "paused";
  const active = recording || paused;
  const showSystem = startedWithSystem ?? captureSetting;

  // Ein Text je Ursache: Ueberlauf und fehlende Spracherkennung betreffen
  // beide Kanaele, sollen aber nur einmal erscheinen.
  const healthTexts = (): string[] => {
    const out: string[] = [];
    for (const channel of [0, 1]) {
      if (channel === 1 && !showSystem) continue;
      const c = health.conditions[channel];
      if (c === "queue_overflow") {
        out.push(t("meetings.record.health.queue_overflow"));
      } else if (c) {
        out.push(
          t(`meetings.record.health.${channel === 0 ? "mic" : "system"}.${c}`),
        );
      }
      const sticky = health.sticky[channel];
      if (sticky) out.push(t(`meetings.record.health.${sticky}`));
    }
    return [...new Set(out)];
  };

  const autoNotesText = (state: AutoNotes): string => {
    if (state.kind === "running") {
      return t("meetings.record.autoNotes.running", {
        step: state.step,
        total: state.total,
      });
    }
    if (state.kind === "done") return t("meetings.record.autoNotes.done");
    return AUTO_NOTES_CODES.has(state.code)
      ? t(`meetings.record.autoNotes.failed.${state.code}`)
      : t("meetings.record.autoNotes.failed.generic");
  };

  return (
    <SettingsGroup title={t("meetings.title")}>
      <div className="px-4 py-3 space-y-3">
        {error && <Alert variant="error">{error}</Alert>}
        {!active && autoNotes && (
          <div data-testid="auto-notes-status" data-state={autoNotes.kind}>
            <Alert
              variant={
                autoNotes.kind === "failed"
                  ? "warning"
                  : autoNotes.kind === "done"
                    ? "success"
                    : "info"
              }
            >
              {autoNotesText(autoNotes)}
            </Alert>
          </div>
        )}

        {!active && (
          <div className="flex gap-2 items-center flex-wrap">
            <Input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t("meetings.record.titlePlaceholder")}
              className="flex-1 min-w-48"
            />
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={captureSetting}
                onChange={(e) =>
                  void updateSetting("meeting_capture_system", e.target.checked)
                }
                className="accent-logo-primary"
                data-testid="capture-system"
              />
              {t("meetings.record.captureSystem")}
            </label>
            {captureSetting && (
              <label
                className="flex items-center gap-2 text-sm"
                title={t("meetings.record.diarizeMicHint")}
              >
                <input
                  type="checkbox"
                  checked={diarizeMic}
                  onChange={(e) => setDiarizeMic(e.target.checked)}
                  className="accent-logo-primary"
                  data-testid="diarize-mic"
                />
                {t("meetings.record.diarizeMic")}
              </label>
            )}
          </div>
        )}

        {!active && eventChoice && (
          <div
            className="flex items-center gap-1.5 text-xs text-text/70"
            data-testid="calendar-chip"
            data-event-key={eventChoice.event.key}
          >
            <span className="rounded-full bg-logo-primary/20 px-2 py-0.5">
              {distinctAttendees(eventChoice.event) > 0
                ? t("meetings.calendar.suggestion.chipWithAttendees", {
                    count: distinctAttendees(eventChoice.event),
                  })
                : t("meetings.calendar.suggestion.chip")}
            </span>
            <button
              type="button"
              onClick={clearEventChoice}
              aria-label={t("meetings.calendar.suggestion.clear")}
              title={t("meetings.calendar.suggestion.clear")}
              className="rounded p-0.5 hover:bg-mid-gray/20 cursor-pointer"
              data-testid="calendar-chip-clear"
            >
              <X width={12} height={12} />
            </button>
          </div>
        )}

        {!active && (
          <div data-testid="record-template">
            <TemplatePicker
              value={templateId}
              onChange={(id) => setTemplateChoice(id)}
              disabled={busy}
            />
          </div>
        )}

        <div className="flex gap-2 items-center flex-wrap">
          {!active ? (
            <Button onClick={openConsent} disabled={busy}>
              {t("meetings.record.start")}
            </Button>
          ) : (
            <>
              <Badge variant={paused ? "secondary" : "success"}>
                {paused ? t("meetings.record.pause") : t("meetings.title")}
              </Badge>
              {paused ? (
                <Button variant="secondary" onClick={resume} disabled={busy}>
                  {t("meetings.record.resume")}
                </Button>
              ) : (
                <Button variant="secondary" onClick={pause} disabled={busy}>
                  {t("meetings.record.pause")}
                </Button>
              )}
              <Button variant="danger" onClick={stop} disabled={busy}>
                {t("meetings.record.stop")}
              </Button>
            </>
          )}
        </div>

        {!active && (hasCalendar || upcoming.length > 0) && (
          <div
            className="rounded-lg border border-mid-gray/20"
            data-testid="upcoming-card"
          >
            <button
              type="button"
              onClick={() => setUpcomingOpen((v) => !v)}
              aria-expanded={upcomingOpen}
              className="flex w-full items-center gap-1.5 px-3 py-2 text-sm font-medium cursor-pointer"
              data-testid="upcoming-toggle"
            >
              {upcomingOpen ? (
                <ChevronDown width={14} height={14} />
              ) : (
                <ChevronRight width={14} height={14} />
              )}
              {t("meetings.calendar.upcoming.title")}
            </button>
            {upcomingOpen &&
              (upcoming.length === 0 ? (
                <p className="px-3 pb-2 text-sm text-text/70">
                  {t("meetings.calendar.upcoming.empty")}
                </p>
              ) : (
                <ul className="px-3 pb-2 space-y-1.5">
                  {upcoming.map((event) => (
                    <li
                      key={event.key}
                      className="flex items-center justify-between gap-2 text-sm"
                      data-testid="upcoming-event"
                    >
                      <div className="min-w-0">
                        <span className="tabular-nums text-text/70">
                          {timeOf(event.starts_at)}–{timeOf(event.ends_at)}
                        </span>{" "}
                        <span className="font-medium break-words">
                          {event.title}
                        </span>
                        {distinctAttendees(event) > 0 && (
                          <span className="text-text/60">
                            {" "}
                            ·{" "}
                            {t("meetings.calendar.upcoming.attendees", {
                              count: distinctAttendees(event),
                            })}
                          </span>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-1.5">
                        {onPrepare && (
                          <BriefButton
                            eventKey={event.key}
                            onOpen={onPrepare}
                            testId="upcoming-brief"
                          />
                        )}
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={() => startFromCard(event)}
                          disabled={busy}
                          data-testid="upcoming-start"
                        >
                          {t("meetings.calendar.upcoming.start")}
                        </Button>
                      </div>
                    </li>
                  ))}
                </ul>
              ))}
          </div>
        )}

        {active && (
          <div data-testid="health-warnings" className="space-y-2">
            {healthTexts().map((text) => (
              <div key={text} data-testid="health-warning">
                <Alert variant="warning">{text}</Alert>
              </div>
            ))}
          </div>
        )}

        {active && (
          <div className="space-y-1.5 pt-1">
            <LevelBar label={t("meetings.record.micLevel")} value={micLevel} />
            {showSystem && (
              <LevelBar
                label={t("meetings.record.systemLevel")}
                value={systemLevel}
              />
            )}
          </div>
        )}

        {active && <MeetingChatNotice testId="recording" />}
      </div>

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
          <MeetingChatNotice testId="consent" />
        </div>
      </Dialog>
    </SettingsGroup>
  );
};
