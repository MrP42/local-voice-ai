// #848: pure logic of the meeting recording pill in the overlay. Only numbers
// and flags come in (which sources record, their RMS level); no audio, no text.

/** Which sources the running meeting records (event `meeting-indicator`). */
export type MeetingSources = { mic: boolean; system: boolean };

/** Bars per source track. */
export const MEETING_BARS = 3;

/**
 * Below this normalised level a source counts as silent and its track rests
 * flat. Roughly -50 dBFS RMS: room noise of a muted or idle microphone.
 */
export const MEETING_QUIET_LEVEL = 0.12;

/** Scale of a resting bar (4 px of the 14 px bar). */
export const MEETING_REST_SCALE = 4 / 14;

// Bar shapes cycled per level update, so a talking source moves even while its
// RMS stays about the same. Centre-weighted, like a small equaliser.
const SHAPES: readonly (readonly number[])[] = [
  [0.62, 1, 0.78],
  [0.9, 0.72, 1],
  [0.7, 0.94, 0.6],
  [1, 0.8, 0.86],
];

/** Maps a channel RMS (0..1) to 0..1; speech sits around 0.3 to 1. */
export function normaliseLevel(rms: number): number {
  if (!Number.isFinite(rms) || rms <= 0) return 0;
  return Math.min(1, Math.sqrt(rms) * 2.2);
}

/**
 * Per-bar vertical scales for one track, or `null` when the source is silent
 * (the track then rests flat and does not move).
 */
export function meetingBarScales(rms: number, tick: number): number[] | null {
  const level = normaliseLevel(rms);
  if (level < MEETING_QUIET_LEVEL) return null;
  const shape = SHAPES[Math.abs(Math.trunc(tick)) % SHAPES.length];
  return shape.map((factor) => {
    const scale =
      MEETING_REST_SCALE + (1 - MEETING_REST_SCALE) * level * factor;
    return Math.round(scale * 1000) / 1000;
  });
}

/** i18n key of the tooltip / aria-label sentence for the active sources. */
export function meetingLabelKey(sources: MeetingSources): string {
  if (sources.mic && sources.system) return "overlay.meeting.label.both";
  if (sources.system) return "overlay.meeting.label.system";
  return "overlay.meeting.label.mic";
}
