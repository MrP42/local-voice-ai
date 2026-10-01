import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { commands, type Meeting, type YoutubeSource } from "@/bindings";
import { timedRange, type TranscriptPlayer } from "../transcriptPlayer";
import {
  YoutubePlayerPanel,
  type YoutubePanelHandle,
} from "./YoutubePlayerPanel";

/** Der Wert von `meetings.source` fuer Besprechungen aus einem YouTube-Link. */
export const YOUTUBE_SOURCE = "youtube";

export interface YoutubeBinding {
  /** Die Quelle, sobald sie geladen ist; `null` bei anderen Besprechungen. */
  source: YoutubeSource | null;
  /** Zeitmarken-Player der Besprechung (ersetzt das Audio); sonst `null`. */
  player: TranscriptPlayer | null;
  /** Der Player-Bereich fuer die Inhaltsspalte; sonst `null`. */
  panel: React.ReactNode;
}

/**
 * Haengt die YouTube-Quelle an eine Besprechung: laedt ihre Angaben (kein Netz,
 * nur die lokale Datenbank), liefert den Player-Bereich und den austauschbaren
 * Zeitmarken-Player (`transcriptPlayer.ts`), der ein Klick auf ein Segment in
 * einen Zeitsprung des YouTube-Players uebersetzt. Bei jeder anderen Besprechung
 * ist alles `null`: die Audio-Wiedergabe bleibt unveraendert.
 */
export function useYoutubeSource(meeting: Meeting): YoutubeBinding {
  const isYoutube = meeting.source === YOUTUBE_SOURCE;
  const [source, setSource] = useState<YoutubeSource | null>(null);
  const handle = useRef<YoutubePanelHandle | null>(null);

  useEffect(() => {
    if (!isYoutube) {
      setSource(null);
      return;
    }
    let cancelled = false;
    void commands.youtubeSourceGet(meeting.id).then((result) => {
      if (!cancelled && result.status === "ok") setSource(result.data ?? null);
    });
    return () => {
      cancelled = true;
    };
  }, [isYoutube, meeting.id]);

  const player = useMemo<TranscriptPlayer | null>(
    () =>
      source
        ? {
            canSeek: true,
            seek: (ms) => handle.current?.seek(ms),
            ...timedRange(
              (ms) => handle.current?.seek(ms),
              () => handle.current?.pause(),
            ),
          }
        : null,
    [source],
  );

  // A3: die Dauer fehlt bei einer neuen YouTube-Besprechung (oEmbed nennt sie
  // nicht); der Player kennt sie nach dem Start und traegt sie einmalig nach.
  const durationMissing = meeting.duration_ms == null;
  const onDuration = useCallback(
    (seconds: number) => {
      if (durationMissing)
        void commands.youtubeSetDuration(meeting.id, seconds);
    },
    [durationMissing, meeting.id],
  );

  const panel = source ? (
    <YoutubePlayerPanel ref={handle} source={source} onDuration={onDuration} />
  ) : null;

  return { source, player, panel };
}
