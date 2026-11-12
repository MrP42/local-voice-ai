import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, events, type Meeting, type SubtitleTrack } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Select } from "../../../ui/Select";
import { useSettings } from "../../../../hooks/useSettings";
import { useRecordingActive } from "../../../../hooks/useRecordingActive";
import { translateMeetingError } from "../meetingErrors";
import { PlaybackCaptureDialog } from "./PlaybackCaptureDialog";
import { translateVariantError } from "./useVariants";

interface YoutubeTranscriptToolsProps {
  meetingId: string;
  /** Eine Fassung ist dazugekommen oder die Transkription ist fertig. */
  onChanged: () => void | Promise<void>;
  /** Die Verarbeitung laeuft (Fortschritt im Kopf): Knoepfe gesperrt. */
  busy: boolean;
  /** Das Transkript hat schon Saetze: Mitschreiben wird nicht mehr angeboten. */
  hasTranscript: boolean;
  /** Zu dieser Besprechung liegt schon eine Aufnahme auf der Platte. */
  hasAudio: boolean;
  /**
   * Die Aufnahme laeuft (der Eintrag ist jetzt `recording`). Liefert `true`, wenn
   * der Player das Video von vorn abspielt; `false`, wenn der Nutzer selbst auf
   * "Abspielen" druecken muss (Player noch nicht bereit).
   */
  onCaptureStarted: (meeting: Meeting) => boolean;
}

/** Zustaende des Systemton-Waechters, bei denen kein Ton ankommt. */
const QUIET_STATES = ["no_data", "digital_zero", "silent"];

/**
 * Transkript einer YouTube-Besprechung im Transkript-Reiter (#537): ohne
 * weitere Software "Beim Abspielen mitschreiben" (der Systemton wird waehrend
 * des Abspielens im eingebetteten Player aufgenommen und transkribiert, kein
 * Download), und mit einem selbst installierten yt-dlp zusaetzlich Untertitel und
 * eigene Transkription (Schalter „privat“). Der Player bleibt in jedem Fall
 * nutzbar.
 */
export const YoutubeTranscriptTools: React.FC<YoutubeTranscriptToolsProps> = ({
  meetingId,
  onChanged,
  busy,
  hasTranscript,
  hasAudio,
  onCaptureStarted,
}) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const recording = useRecordingActive();
  const capturing = recording.active && recording.meetingId === meetingId;
  const recordingElsewhere = recording.active && !capturing;
  const [consentOpen, setConsentOpen] = useState(false);
  const [starting, setStarting] = useState(false);
  // „Abbrechen“ ist abgeschickt: der Stopp läuft, der Start meldet sich gleich zurück.
  const [aborting, setAborting] = useState(false);
  // Escape, X und Hintergrund können im selben Takt mehrfach ankommen, bevor der
  // Zustand neu gezeichnet ist: der Stopp wird trotzdem nur einmal abgeschickt.
  const abortSent = useRef(false);
  // Der Player konnte das Video nicht selbst starten: der Nutzer drueckt "Abspielen".
  const [playHint, setPlayHint] = useState(false);
  // Der Systemton liefert seit einiger Zeit nichts (Waechter, Kanal 1).
  const [quiet, setQuiet] = useState(false);
  const enabled = getSetting("meeting_youtube_private") ?? false;
  const toolPath = getSetting("meeting_youtube_tool_path") ?? "";
  const [found, setFound] = useState<boolean | null>(null);
  const [working, setWorking] = useState<"tracks" | "fetch" | "own" | null>(
    null,
  );
  const [tracks, setTracks] = useState<SubtitleTrack[] | null>(null);
  const [pick, setPick] = useState("");
  const [message, setMessage] = useState<{
    kind: "info" | "error";
    text: string;
  } | null>(null);

  // Das Programm wird nur gesucht, wenn der Schalter an ist (kein Start ohne ihn).
  useEffect(() => {
    if (!enabled) {
      setFound(false);
      return;
    }
    let cancelled = false;
    setFound(null);
    void commands.youtubeToolDetect(null).then((result) => {
      if (!cancelled) setFound(result.status === "ok" && result.data.found);
    });
    return () => {
      cancelled = true;
    };
  }, [enabled, toolPath]);

  useEffect(() => {
    setTracks(null);
    setMessage(null);
    setPlayHint(false);
  }, [meetingId]);

  // Kein Ton waehrend des Mitschreibens: die Oberflaeche fragt nach, ob das Video
  // laeuft. Die Meldung geht von selbst, sobald wieder Ton kommt (`recovered`).
  useEffect(() => {
    if (!capturing) {
      setQuiet(false);
      setPlayHint(false);
      return;
    }
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (
        payload.kind !== "health" ||
        payload.meeting_id !== meetingId ||
        payload.channel !== 1
      ) {
        return;
      }
      if (QUIET_STATES.includes(payload.state)) setQuiet(true);
      else if (payload.state === "recovered") setQuiet(false);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [capturing, meetingId]);

  const trackLabel = (track: SubtitleTrack) =>
    t(
      track.auto
        ? "meetings.variants.youtube.trackAuto"
        : "meetings.variants.youtube.trackManual",
      { name: `${track.name} [${track.language}]` },
    );

  const fail = (raw: string) =>
    setMessage({ kind: "error", text: translateVariantError(raw, t) });

  const fetchTrack = async (track: SubtitleTrack) => {
    setWorking("fetch");
    const result = await commands.youtubeSubtitlesFetch(meetingId, track);
    setWorking(null);
    if (result.status !== "ok") {
      fail(result.error);
      return;
    }
    setTracks(null);
    setMessage({
      kind: "info",
      text: t("meetings.variants.youtube.loaded", { name: track.name }),
    });
    await onChanged();
  };

  const findTracks = async () => {
    setWorking("tracks");
    setMessage(null);
    const result = await commands.youtubeSubtitleTracks(meetingId);
    setWorking(null);
    if (result.status !== "ok") {
      fail(result.error);
      return;
    }
    const list = result.data ?? [];
    if (list.length === 0) {
      setMessage({
        kind: "info",
        text: t("meetings.variants.youtube.noSubtitles"),
      });
      return;
    }
    if (list.length === 1) {
      await fetchTrack(list[0]);
      return;
    }
    setTracks(list);
    setPick((list.find((x) => x.recommended) ?? list[0]).language);
  };

  const startCapture = async () => {
    setStarting(true);
    setAborting(false);
    abortSent.current = false;
    setMessage(null);
    const result = await commands.meetingsStartPlaybackCapture(meetingId, true);
    setStarting(false);
    setAborting(false);
    setConsentOpen(false);
    if (result.status !== "ok") {
      const code = result.error.split(":")[0].trim();
      // Vom Nutzer abgebrochen (oder der Eintrag wurde währenddessen gelöscht):
      // kein Fehler, der Eintrag ist wie vorher.
      if (code === "start_cancelled" || code === "meeting_not_found") return;
      setMessage({
        kind: "error",
        text:
          code === "target_not_empty"
            ? t("meetings.variants.youtube.capture.errors.notReady")
            : translateMeetingError(result.error, t),
      });
      return;
    }
    setPlayHint(!onCaptureStarted(result.data));
  };

  // Der laufende Start wird mit demselben Stopp abgebrochen wie eine Aufnahme:
  // das Backend setzt den Abbruchmerker, der Start räumt auf und meldet sich mit
  // `start_cancelled` zurück (siehe `startCapture`).
  const abortStart = async () => {
    if (abortSent.current) return;
    abortSent.current = true;
    setAborting(true);
    const result = await commands.meetingsStop();
    if (result.status !== "ok") {
      abortSent.current = false;
      setAborting(false);
    }
  };

  const startOwn = async () => {
    setWorking("own");
    setMessage(null);
    const result = await commands.youtubeOwnTranscription(meetingId, null);
    setWorking(null);
    if (result.status !== "ok") {
      fail(result.error);
      return;
    }
    setMessage({ kind: "info", text: t("meetings.variants.youtube.ownDone") });
    await onChanged();
  };

  if (found === null) return null;
  const locked = busy || working !== null;
  // Mitschreiben geht nur in einen Eintrag ohne Transkript und ohne Aufnahme.
  const canCapture = !hasTranscript && !hasAudio && !capturing;
  const captureLocked = locked || starting || recordingElsewhere;
  const captureButton = (
    <Button
      size="sm"
      variant={found ? "secondary" : "primary"}
      data-testid="yt-capture"
      disabled={captureLocked}
      title={
        recordingElsewhere
          ? t("meetings.variants.youtube.capture.elsewhere")
          : undefined
      }
      onClick={() => {
        setMessage(null);
        setConsentOpen(true);
      }}
    >
      {starting
        ? t("meetings.variants.youtube.capture.starting")
        : t("meetings.variants.youtube.capture.button")}
    </Button>
  );

  return (
    <div
      role="group"
      aria-label={t("meetings.variants.youtube.tools")}
      className="space-y-1.5"
      data-testid="yt-transcript-tools"
    >
      {capturing && (
        <div
          className="space-y-1"
          role="status"
          data-testid="yt-capture-running"
        >
          <p className="text-xs text-text/70">
            {t("meetings.variants.youtube.capture.running")}
          </p>
          {playHint && (
            <p
              className="text-xs font-medium"
              data-testid="yt-capture-play-hint"
            >
              {t("meetings.variants.youtube.capture.playHint")}
            </p>
          )}
          {quiet && (
            <p
              className="text-xs text-yellow-400"
              data-testid="yt-capture-quiet"
            >
              {t("meetings.variants.youtube.capture.quiet")}
            </p>
          )}
        </div>
      )}
      {canCapture && !found && (
        <div className="space-y-1" data-testid="yt-capture-block">
          {captureButton}
          <p className="text-xs text-text/70" data-testid="yt-capture-explain">
            {t("meetings.variants.youtube.capture.explain")}
          </p>
        </div>
      )}
      {found === false && (
        <p className="text-[11px] text-text/50" data-testid="yt-tool-hint">
          {t("meetings.variants.youtube.needTool")}
        </p>
      )}
      {found === true && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="secondary"
            data-testid="yt-subtitles"
            disabled={locked}
            onClick={() => void findTracks()}
          >
            {working === "tracks" || working === "fetch"
              ? t("meetings.variants.youtube.subtitlesBusy")
              : t("meetings.variants.youtube.subtitles")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            data-testid="yt-own-transcription"
            disabled={locked}
            onClick={() => void startOwn()}
          >
            {working === "own"
              ? t("meetings.variants.youtube.ownBusy")
              : t("meetings.variants.youtube.own")}
          </Button>
          {canCapture && captureButton}
        </div>
      )}
      {canCapture && found === true && (
        <p className="text-xs text-text/70" data-testid="yt-capture-explain">
          {t("meetings.variants.youtube.capture.explain")}
        </p>
      )}
      {found && tracks && (
        <div
          className="flex flex-wrap items-end gap-2"
          data-testid="yt-track-pick"
        >
          <div className="flex min-w-[12rem] flex-col gap-1 text-xs text-text/70">
            <span>{t("meetings.variants.youtube.pick")}</span>
            <div data-testid="yt-track-select">
              <Select
                value={pick || null}
                options={tracks.map((track) => ({
                  value: track.language,
                  label: trackLabel(track),
                }))}
                ariaLabel={t("meetings.variants.youtube.pick")}
                onChange={(value) => value && setPick(value)}
              />
            </div>
          </div>
          <Button
            size="sm"
            data-testid="yt-track-load"
            disabled={locked}
            onClick={() => {
              const chosen = tracks.find((x) => x.language === pick);
              if (chosen) void fetchTrack(chosen);
            }}
          >
            {t("meetings.variants.youtube.load")}
          </Button>
        </div>
      )}
      {message && (
        <p
          role={message.kind === "error" ? "alert" : "status"}
          data-testid="yt-message"
          className={`text-xs ${message.kind === "error" ? "text-red-400" : "text-text/70"}`}
        >
          {message.text}
        </p>
      )}
      <PlaybackCaptureDialog
        open={consentOpen}
        onOpenChange={setConsentOpen}
        busy={starting}
        aborting={aborting}
        onConfirm={() => void startCapture()}
        onAbort={() => void abortStart()}
      />
    </div>
  );
};
