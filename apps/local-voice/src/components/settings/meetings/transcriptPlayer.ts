import type { RefObject } from "react";
import type { AudioPlayerHandle } from "../../ui/AudioPlayer";

/**
 * Der Player, den ein Klick auf eine Zeitmarke im Transkript anspricht. Heute
 * sind das die Audio-Player der Besprechung; ein YouTube-Player (#66) setzt
 * spaeter eine eigene Umsetzung ein, ohne dass das Transkript davon weiss.
 */
export interface TranscriptPlayer {
  /** Gibt es etwas, in das man springen kann (Audio da, Video geladen)? */
  readonly canSeek: boolean;
  /** Ab `ms` abspielen. `channel` waehlt die Spur, falls es mehrere gibt. */
  seek(ms: number, channel?: number): void;
}

/** Kein Player: Zeitmarken sind reiner Text. */
export const NO_PLAYER: TranscriptPlayer = { canSeek: false, seek: () => {} };

/**
 * Audio-Umsetzung: Kanal 1 (Gegenseite) liegt in der Systemaufnahme, alles
 * andere (Mikrofon, Import als Mischkanal) im Mikrofon-/Import-Player.
 */
export function audioTranscriptPlayer(
  mic: RefObject<AudioPlayerHandle | null>,
  system: RefObject<AudioPlayerHandle | null>,
  hasAudio: boolean,
): TranscriptPlayer {
  return {
    canSeek: hasAudio,
    seek(ms, channel) {
      const player =
        channel === 1 && system.current
          ? system.current
          : (mic.current ?? system.current);
      player?.playAt(ms / 1000);
    },
  };
}
