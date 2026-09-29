import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { commands, events, type MeetingPromptPayload } from "@/bindings";
import { syncLanguageFromSettings } from "@/i18n";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { MeetingChatNotice } from "@/components/settings/meetings/MeetingChatNotice";
import { translateMeetingError } from "@/components/settings/meetings/meetingErrors";

type Step = "offer" | "consent";

/** Wie oft die Zeile „beginnt in … min“ nachgezogen wird. */
const TICK_MS = 15_000;

/** Minuten bis zum Beginn, aufgerundet; negativ = laeuft schon. */
export const minutesUntil = (startsAt: number, now: number): number =>
  Math.ceil((startsAt - now) / 60_000);

/**
 * Hinweisfenster fuer eine anstehende Besprechung (M5-P5b). Zustand 1 kuendigt
 * sie an und bietet die Aufnahme an, Zustand 2 holt die Einwilligung ein
 * (§ 201 StGB) - ohne Haekchen startet nichts. Das Fenster hat keinen Fokus;
 * bedient wird es mit der Maus.
 *
 * Der Inhalt liegt im Backend (`meeting_prompt_current`); ein Ereignis meldet
 * nur, dass es einen neuen gibt. So geht nichts verloren, wenn das Fenster
 * erst nach dem Ereignis fertig geladen ist.
 */
const MeetingPrompt: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const [payload, setPayload] = useState<MeetingPromptPayload | null>(null);
  const [step, setStep] = useState<Step>("offer");
  const [agreed, setAgreed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const rootRef = useRef<HTMLDivElement>(null);
  const captureSystem = getSetting("meeting_capture_system") ?? true;

  const load = useCallback(async () => {
    const result = await commands.meetingPromptCurrent();
    const next = result ?? null;
    setPayload((previous) => {
      if (next?.prompt_id !== previous?.prompt_id) {
        setStep("offer");
        setAgreed(false);
        setError(null);
        setBusy(false);
      }
      return next;
    });
  }, []);

  useEffect(() => {
    void syncLanguageFromSettings();
    void load();
    const un = events.meetingPromptEvent.listen((e) => {
      if (e.payload.kind === "show") void load();
      else setPayload(null);
    });
    const timer = setInterval(() => setNow(Date.now()), TICK_MS);
    return () => {
      void un.then((f) => f());
      clearInterval(timer);
    };
  }, [load]);

  // Hoehe melden, sobald sich der Inhalt geaendert hat: erst danach zeigt das
  // Backend das Fenster (kein leeres Aufblitzen, keine falsche Groesse).
  useLayoutEffect(() => {
    if (!payload || !rootRef.current) return;
    void commands.meetingPromptReady(
      Math.ceil(rootRef.current.getBoundingClientRect().height),
    );
  }, [payload, step, error]);

  if (!payload) return <div ref={rootRef} />;

  const event = payload.event;
  const title =
    event?.title ?? payload.app_label ?? t("meetings.prompt.noEvent");
  const minutes = event ? minutesUntil(event.starts_at, now) : null;

  const whenText = (): string | null => {
    if (minutes === null) return null;
    if (minutes > 0) return t("meetings.prompt.startsIn", { count: minutes });
    if (minutes === 0) return t("meetings.prompt.startsNow");
    return t("meetings.prompt.runningSince", { count: -minutes });
  };

  const start = async () => {
    if (!agreed || busy) return; // ohne Haekchen kein Aufruf
    setBusy(true);
    setError(null);
    const result = await commands.meetingsStartFromEvent(
      event?.key ?? null,
      null,
      true,
      captureSystem,
      null,
      "prompt",
    );
    setBusy(false);
    if (result.status === "error") {
      setError(translateMeetingError(result.error, t));
      return;
    }
    setPayload(null);
  };

  const later = () => {
    void commands.meetingPromptDismiss(payload.prompt_id, "later");
  };

  const join = () => {
    if (event) void commands.calendarOpenJoinUrl(event.key);
  };

  return (
    <div
      ref={rootRef}
      data-testid="meeting-prompt"
      data-step={step}
      className="w-full box-border p-4 space-y-3 bg-background text-text border border-mid-gray/30 select-none"
    >
      {step === "offer" ? (
        <>
          <div className="space-y-0.5">
            <p
              className="text-base font-semibold leading-snug break-words"
              data-testid="prompt-title"
            >
              {title}
            </p>
            <p className="text-sm text-text/70" data-testid="prompt-when">
              {[
                whenText(),
                payload.attendee_count > 0
                  ? t("meetings.prompt.attendees", {
                      count: payload.attendee_count,
                    })
                  : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              onClick={() => setStep("consent")}
              data-testid="prompt-start"
            >
              {t("meetings.prompt.start")}
            </Button>
            {event?.join_url && (
              <Button
                variant="secondary"
                onClick={join}
                data-testid="prompt-join"
              >
                {t("meetings.prompt.join")}
              </Button>
            )}
            <Button variant="ghost" onClick={later} data-testid="prompt-later">
              {t("meetings.prompt.later")}
            </Button>
          </div>
        </>
      ) : (
        <>
          <div className="space-y-1">
            <p className="text-base font-semibold">
              {t("meetings.consent.title")}
            </p>
            <p className="text-xs text-text/70 break-words">{title}</p>
          </div>
          <p className="text-sm text-text/80">{t("meetings.consent.body")}</p>
          <MeetingChatNotice testId="prompt" />
          <label className="flex items-start gap-2 text-sm cursor-pointer">
            <input
              type="checkbox"
              checked={agreed}
              onChange={(e) => setAgreed(e.target.checked)}
              className="mt-1 accent-logo-primary"
              data-testid="prompt-consent"
            />
            <span>{t("meetings.consent.confirm")}</span>
          </label>
          <label className="flex items-center gap-2 text-sm cursor-pointer">
            <input
              type="checkbox"
              checked={captureSystem}
              onChange={(e) =>
                void updateSetting("meeting_capture_system", e.target.checked)
              }
              className="accent-logo-primary"
              data-testid="prompt-capture-system"
            />
            <span>{t("meetings.record.captureSystem")}</span>
          </label>
          {error && (
            <p
              className="text-sm text-red-500"
              role="alert"
              data-testid="prompt-error"
            >
              {error}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              onClick={() => void start()}
              disabled={!agreed || busy}
              data-testid="prompt-confirm"
            >
              {busy
                ? t("meetings.prompt.starting")
                : t("meetings.prompt.confirm")}
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                setStep("offer");
                setAgreed(false);
                setError(null);
              }}
              disabled={busy}
              data-testid="prompt-back"
            >
              {t("meetings.prompt.back")}
            </Button>
          </div>
        </>
      )}
    </div>
  );
};

export default MeetingPrompt;
