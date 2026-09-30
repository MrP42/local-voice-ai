import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type SubtitleTrack } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Select } from "../../../ui/Select";
import { useSettings } from "../../../../hooks/useSettings";
import { translateVariantError } from "./useVariants";

interface YoutubeTranscriptToolsProps {
  meetingId: string;
  /** Eine Fassung ist dazugekommen oder die Transkription ist fertig. */
  onChanged: () => void | Promise<void>;
  /** Die Verarbeitung laeuft (Fortschritt im Kopf): Knoepfe gesperrt. */
  busy: boolean;
}

/**
 * Untertitel und eigene Transkription einer YouTube-Besprechung im Transkript-
 * Reiter. Beides geht nur mit Schalter „privat“ und einem selbst installierten
 * yt-dlp; sonst steht hier der Hinweis, was zu tun ist. Der Player bleibt in
 * jedem Fall nutzbar.
 */
export const YoutubeTranscriptTools: React.FC<YoutubeTranscriptToolsProps> = ({
  meetingId,
  onChanged,
  busy,
}) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
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
  }, [meetingId]);

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

  return (
    <div
      role="group"
      aria-label={t("meetings.variants.youtube.tools")}
      className="space-y-1.5"
      data-testid="yt-transcript-tools"
    >
      {!found ? (
        <p className="text-xs text-text/70" data-testid="yt-tool-hint">
          {t("meetings.variants.youtube.needTool")}
        </p>
      ) : (
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
        </div>
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
    </div>
  );
};
