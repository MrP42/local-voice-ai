import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play, Square } from "lucide-react";
import { commands } from "@/bindings";
import { useNow } from "@/hooks/useMeetingJobs";
import {
  JOB_ERROR_KEYS,
  countsAudio,
  etaText,
  formatClock,
  isIndeterminate,
  liveElapsedMs,
  liveEtaMs,
  percentOf,
  type LiveProgress,
} from "@/lib/meetingJobs";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { IconAction } from "../../ui/IconAction";

/** Text der Restdauer; `null` zeigt nichts (pausiert). */
const useEtaLabel = (progress: LiveProgress, now: number) => {
  const { t } = useTranslation();
  if (progress.state === "paused") return null;
  const eta = liveEtaMs(progress, now);
  if (eta === null) {
    return progress.total > 0 && !isIndeterminate(progress)
      ? t("meetings.progress.eta.calculating")
      : null;
  }
  const text = etaText(eta);
  switch (text.unit) {
    case "few":
      return t("meetings.progress.eta.few");
    case "seconds":
      return t("meetings.progress.eta.seconds", { count: text.seconds });
    case "minutes":
      return t("meetings.progress.eta.minutes", { count: text.minutes });
    case "hours":
      return t("meetings.progress.eta.hours", {
        hours: text.hours,
        minutes: text.minutes,
      });
  }
};

/** Der Balken selbst: Prozent, oder unbestimmt (pulsierend) ohne Groesse. */
const Bar: React.FC<{ progress: LiveProgress; thin?: boolean }> = ({
  progress,
  thin = false,
}) => {
  const { t } = useTranslation();
  const percent = percentOf(progress);
  const indeterminate = isIndeterminate(progress);
  return (
    <div
      role="progressbar"
      aria-label={t(`meetings.progress.phase.${progress.phase}`)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate || percent === null ? undefined : percent}
      data-testid="job-bar-track"
      className={`w-full overflow-hidden rounded-full bg-mid-gray/20 ${
        thin ? "h-1.5" : "h-2"
      }`}
    >
      <div
        data-testid="job-bar-fill"
        className={`h-full rounded-full bg-logo-primary ${
          indeterminate
            ? "w-full animate-pulse opacity-60"
            : "transition-[width] duration-500"
        } ${progress.state === "paused" ? "opacity-50" : ""}`}
        style={indeterminate ? undefined : { width: `${percent ?? 0}%` }}
      />
    </div>
  );
};

/**
 * Schmaler Balken fuer die Liste (und die Reiter Notizen/Protokoll): Phase,
 * Prozent und Restdauer. Klicks darauf oeffnen nicht die Besprechung.
 */
export const JobBar: React.FC<{
  progress: LiveProgress;
  className?: string;
  /** U7: die Warteschlange hat den Import wegen einer Aufnahme angehalten. */
  heldForRecording?: boolean;
}> = ({ progress, className = "w-44", heldForRecording = false }) => {
  const { t } = useTranslation();
  const now = useNow(progress.state !== "paused");
  const eta = useEtaLabel(progress, now);
  const percent = percentOf(progress);
  const indeterminate = isIndeterminate(progress);
  const paused = progress.state === "paused" || progress.state === "pausing";
  const stateNote =
    progress.state === "running"
      ? null
      : heldForRecording && paused
        ? t("meetings.queue.heldForRecording")
        : t(`meetings.progress.state.${progress.state}`);
  return (
    <div
      data-testid="job-bar"
      data-phase={progress.phase}
      data-state={progress.state}
      className={`space-y-0.5 ${className}`}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-baseline justify-between gap-2 text-[11px] text-text/70">
        <span className="truncate">
          {t(`meetings.progress.phase.${progress.phase}`)}
        </span>
        {!indeterminate && percent !== null && (
          <span className="tabular-nums" data-testid="job-bar-percent">
            {t("meetings.progress.percent", { percent })}
          </span>
        )}
      </div>
      <Bar progress={progress} thin />
      <div
        className="truncate text-[11px] tabular-nums text-text/50"
        data-testid="job-bar-note"
      >
        {stateNote ?? eta ?? " "}
      </div>
    </div>
  );
};

interface JobPanelProps {
  progress: LiveProgress;
  /** U7: die Warteschlange hat den Import wegen einer Aufnahme angehalten. */
  heldForRecording?: boolean;
}

/**
 * Statuszeile einer laufenden Verarbeitung in der Bedienspalte: Phase, Balken
 * mit Prozent, Pausieren / Fortsetzen und Stoppen als Symbole (Stoppen mit
 * Rueckfrage im Dialog), darunter Menge, Laufzeit und geschaetzte Restdauer. Alles Sichtbare kommt aus
 * dem Backend-Zustand, nicht aus lokalem Wissen: beim Reiterwechsel oder Neu-
 * Oeffnen sieht man denselben Stand.
 */
export const JobPanel: React.FC<JobPanelProps> = ({
  progress,
  heldForRecording = false,
}) => {
  const { t } = useTranslation();
  const now = useNow(progress.state !== "paused");
  const eta = useEtaLabel(progress, now);
  const percent = percentOf(progress);
  const indeterminate = isIndeterminate(progress);
  const [confirmStop, setConfirmStop] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const meetingId = progress.meeting_id;

  const describe = (code: string) => {
    const head = code.split(":")[0].trim();
    const key = JOB_ERROR_KEYS[head];
    return key ? t(key) : code;
  };

  const run = async (
    call: () => Promise<{ status: "ok" | "error"; error?: unknown }>,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await call();
      if (result.status === "error") setError(describe(String(result.error)));
    } finally {
      setBusy(false);
    }
  };

  const holding = progress.state === "pausing" || progress.state === "paused";
  const stopping = progress.state === "stopping";
  const pauseDisabled = busy || stopping || (!holding && !progress.pausable);
  const generating =
    progress.phase === "notes" ||
    progress.phase === "minutes" ||
    progress.phase === "translation";

  const statusText =
    progress.state === "running"
      ? null
      : heldForRecording && holding
        ? t("meetings.queue.heldForRecording")
        : t(`meetings.progress.state.${progress.state}`);

  return (
    <div
      data-testid="job-panel"
      data-phase={progress.phase}
      data-state={progress.state}
      className="space-y-1 rounded-md border border-mid-gray/20 px-2.5 py-1.5"
    >
      {/* Eine Zeile: Phase, Balken, Prozent, Pause und Stopp als Symbole. */}
      <div className="flex items-center gap-2">
        <span
          className="max-w-[45%] shrink-0 truncate text-sm font-medium"
          data-testid="job-phase"
        >
          {t(`meetings.progress.phase.${progress.phase}`)}
        </span>
        {statusText && (
          <span
            className="shrink-0 truncate text-xs text-text/60"
            data-testid="job-state"
          >
            {statusText}
          </span>
        )}
        <div className="min-w-[2rem] flex-1">
          <Bar progress={progress} thin />
        </div>
        {!indeterminate && percent !== null && (
          <span
            className="shrink-0 text-xs tabular-nums text-text/70"
            data-testid="job-percent"
          >
            {t("meetings.progress.percent", { percent })}
          </span>
        )}
        <IconAction
          icon={holding ? Play : Pause}
          label={
            holding
              ? t("meetings.progress.resume")
              : t("meetings.progress.pause")
          }
          description={
            !holding && !progress.pausable && !stopping
              ? t("meetings.progress.notPausable")
              : holding
                ? t("meetings.recorder.resumeHint")
                : t("meetings.progress.pauseHint")
          }
          testId={holding ? "job-resume" : "job-pause"}
          disabled={pauseDisabled}
          onClick={() =>
            void run(() =>
              holding
                ? commands.meetingsJobResume(meetingId)
                : commands.meetingsJobPause(meetingId),
            )
          }
        />
        <IconAction
          icon={Square}
          label={t("meetings.progress.stop")}
          description={t("meetings.progress.stopHint")}
          testId="job-stop"
          iconClassName="text-red-500"
          disabled={busy || stopping}
          onClick={() => setConfirmStop(true)}
        />
      </div>
      <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] tabular-nums text-text/60">
        {progress.total > 0 &&
          (countsAudio(progress) ? (
            <span data-testid="job-amount">
              {t("meetings.progress.audioOf", {
                done: formatClock(progress.done),
                total: formatClock(progress.total),
              })}
            </span>
          ) : (
            <span data-testid="job-amount">
              {t("meetings.progress.stepsOf", {
                done: progress.done,
                total: progress.total,
              })}
            </span>
          ))}
        <span data-testid="job-elapsed">
          {t("meetings.progress.elapsed", {
            time: formatClock(liveElapsedMs(progress, now)),
          })}
        </span>
        {eta && <span data-testid="job-eta">{eta}</span>}
      </div>
      {error && (
        <div data-testid="job-error">
          <Alert variant="error">{error}</Alert>
        </div>
      )}
      <Dialog
        open={confirmStop}
        onOpenChange={setConfirmStop}
        title={t("meetings.progress.stopTitle")}
        closeLabel={t("meetings.progress.stopCancel")}
        footer={
          <>
            <Button
              variant="secondary"
              data-testid="job-stop-cancel"
              onClick={() => setConfirmStop(false)}
            >
              {t("meetings.progress.stopCancel")}
            </Button>
            <Button
              variant="danger"
              data-testid="job-stop-confirm"
              onClick={() => {
                setConfirmStop(false);
                void run(() => commands.meetingsJobStop(meetingId));
              }}
            >
              {t("meetings.progress.stopConfirm")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {generating
            ? t("meetings.progress.stopBodyGenerate")
            : t("meetings.progress.stopBody")}
        </p>
      </Dialog>
    </div>
  );
};
