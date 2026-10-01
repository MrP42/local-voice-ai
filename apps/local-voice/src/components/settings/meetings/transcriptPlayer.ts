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
  /**
   * Eine Hoerprobe: von `startMs` bis `endMs` abspielen und dann anhalten.
   * `onEnd` kommt genau einmal, wenn die Probe endet: am Ende, bei `stop()`,
   * bei Pause von aussen oder wenn etwas anderes die Probe abloest.
   */
  playRange(
    startMs: number,
    endMs: number,
    channel: number | undefined,
    onEnd: () => void,
  ): void;
  /** Haelt jede laufende Wiedergabe dieses Players an. */
  stop(): void;
}

/** Kein Player: Zeitmarken sind reiner Text. */
export const NO_PLAYER: TranscriptPlayer = {
  canSeek: false,
  seek: () => {},
  playRange: (_start, _end, _channel, onEnd) => onEnd(),
  stop: () => {},
};

/**
 * Audio-Umsetzung: Kanal 1 (Gegenseite) liegt in der Systemaufnahme, alles
 * andere (Mikrofon, Import als Mischkanal) im Mikrofon-/Import-Player.
 */
export function audioTranscriptPlayer(
  mic: RefObject<AudioPlayerHandle | null>,
  system: RefObject<AudioPlayerHandle | null>,
  hasAudio: boolean,
): TranscriptPlayer {
  const pick = (channel: number | undefined) =>
    channel === 1 && system.current
      ? system.current
      : (mic.current ?? system.current);
  return {
    canSeek: hasAudio,
    seek(ms, channel) {
      pick(channel)?.playAt(ms / 1000);
    },
    playRange(startMs, endMs, channel, onEnd) {
      const player = pick(channel);
      if (!player) {
        onEnd();
        return;
      }
      player.playRange(startMs / 1000, endMs / 1000, onEnd);
    },
    stop() {
      mic.current?.stop();
      system.current?.stop();
    },
  };
}

/**
 * Zeitgesteuerte Hoerprobe fuer Player ohne eigenes Zeitgefuehl (YouTube):
 * springt mit `seek` an die Stelle und ruft nach der Laenge der Probe `stop`.
 * `onEnd` kommt genau einmal (Ablauf der Zeit, naechste Probe, `cancel`).
 */
export function timedRange(
  seek: (ms: number) => void,
  stop: () => void,
): Pick<TranscriptPlayer, "playRange" | "stop"> {
  let current: {
    timer: ReturnType<typeof setTimeout>;
    onEnd: () => void;
  } | null = null;
  const finish = () => {
    const running = current;
    current = null;
    if (!running) return;
    clearTimeout(running.timer);
    running.onEnd();
  };
  return {
    playRange(startMs, endMs, _channel, onEnd) {
      finish();
      seek(startMs);
      const timer = setTimeout(
        () => {
          stop();
          finish();
        },
        Math.max(0, endMs - startMs),
      );
      current = { timer, onEnd };
    },
    stop() {
      stop();
      finish();
    },
  };
}
