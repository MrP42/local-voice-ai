import type { MeetingSlide, MeetingSlidesEvent } from "@/bindings";

/**
 * D4 (#70, M7 = #69): Folien einer Besprechung. Reine Funktionen ohne React und
 * Tauri: Rechnung (welche Folie gilt zu welcher Zeit, wo stehen die Marken im
 * Transkript) und Anzeige bleiben getrennt.
 */

/** Dateiendungen, bei denen die Quelle einer Besprechung ein Video sein kann. */
export const VIDEO_EXTENSIONS = [
  "mp4",
  "mkv",
  "mov",
  "webm",
  "m4v",
  "avi",
  "wmv",
];

const extensionOf = (path: string) =>
  path.split(".").pop()?.toLowerCase() ?? "";

export const isVideoPath = (path: string | null | undefined): boolean =>
  !!path && VIDEO_EXTENSIONS.includes(extensionOf(path));

/**
 * Kann fuer diese Besprechung die Folienerkennung laufen? Sie braucht eine
 * Videodatei als Quelle des Imports; Audio, Untertitel und YouTube ohne Bild
 * haben keine Folien. Die endgueltige Pruefung (Datei da, Videospur) macht das
 * Backend und meldet sie als Code zurueck.
 */
export const canDetectSlides = (meeting: {
  source: string;
  source_path: string | null;
}): boolean => meeting.source === "import" && isVideoPath(meeting.source_path);

/** Der Beginn der ersten Sichtung einer Folie im Video (ms). */
export const slideStartMs = (slide: MeetingSlide): number =>
  slide.occurrences.length > 0
    ? Math.min(...slide.occurrences.map((o) => o.start_ms))
    : 0;

/** Die sichtbaren Folien (nicht ausgeblendet), nach Nummer. */
export const visibleSlides = (slides: MeetingSlide[]): MeetingSlide[] =>
  slides.filter((s) => !s.hidden).sort((a, b) => a.number - b.number);

/**
 * Die Folie, die zur Abspielzeit `ms` zu sehen ist: ein Zeitbereich, der `ms`
 * enthaelt; sonst (Luecke zwischen zwei Bereichen, etwa ein ausgeblendetes
 * Sprecherbild) die zuletzt begonnene sichtbare Folie. Vor der ersten Folie
 * gibt es keine (`null`). Ausgeblendete Folien zaehlen nicht.
 */
export const slideAt = (slides: MeetingSlide[], ms: number): string | null => {
  let inside: { id: string; start: number } | null = null;
  let latest: { id: string; start: number } | null = null;
  for (const slide of slides) {
    if (slide.hidden) continue;
    for (const o of slide.occurrences) {
      if (o.start_ms > ms) continue;
      if (!latest || o.start_ms > latest.start) {
        latest = { id: slide.id, start: o.start_ms };
      }
      if (ms < o.end_ms && (!inside || o.start_ms > inside.start)) {
        inside = { id: slide.id, start: o.start_ms };
      }
    }
  }
  return (inside ?? latest)?.id ?? null;
};

/** Eine Folienmarke im Transkript: ab `startMs` ist diese Folie zu sehen. */
export interface SlideMark {
  slideId: string;
  number: number;
  startMs: number;
}

/** Alle Marken sichtbarer Folien (je Zeitbereich eine), nach Zeit. */
export const slideMarks = (slides: MeetingSlide[]): SlideMark[] =>
  visibleSlides(slides)
    .flatMap((slide) =>
      slide.occurrences.map((o) => ({
        slideId: slide.id,
        number: slide.number,
        startMs: o.start_ms,
      })),
    )
    .sort((a, b) => a.startMs - b.startMs || a.number - b.number);

/**
 * Ordnet jede Marke dem ersten Segment zu, das nicht vor ihr beginnt: die Marke
 * steht im Transkript ueber diesem Segment. Marken nach dem letzten Segment
 * bekommt das letzte. Schluessel ist `segment_index`.
 */
export const marksBySegment = (
  segments: ReadonlyArray<{ segment_index: number; start_ms: number }>,
  marks: SlideMark[],
): Map<number, SlideMark[]> => {
  const result = new Map<number, SlideMark[]>();
  if (segments.length === 0) return result;
  const ordered = [...segments].sort((a, b) => a.start_ms - b.start_ms);
  let cursor = 0;
  for (const mark of marks) {
    while (
      cursor < ordered.length - 1 &&
      ordered[cursor].start_ms < mark.startMs
    ) {
      cursor += 1;
    }
    const index = ordered[cursor].segment_index;
    const list = result.get(index);
    if (list) list.push(mark);
    else result.set(index, [mark]);
  }
  return result;
};

/** Bild-Pfad einer Folie: Besprechungsordner (absolut) + relativer Pfad. */
export const slideFilePath = (dir: string, relative: string): string =>
  `${dir.replace(/[\\/]+$/, "")}/${relative.replace(/^[\\/]+/, "")}`;

/** Fehlercodes der Folienerkennung als i18n-Schluessel (`meetings.slides.errors.*`). */
export const SLIDE_ERROR_KEYS: Record<string, string> = {
  recording_active: "meetings.slides.errors.recordingActive",
  meeting_not_finished: "meetings.slides.errors.notFinished",
  meeting_not_found: "meetings.errors.notFound",
  job_busy: "meetings.progress.errors.busy",
  slides_no_video: "meetings.slides.errors.noVideo",
  slides_video_missing: "meetings.slides.errors.videoMissing",
  slides_ffmpeg_missing: "meetings.slides.errors.ffmpegMissing",
  slides_ffmpeg_failed: "meetings.slides.errors.ffmpegFailed",
  slides_disk_full: "meetings.slides.errors.diskFull",
  slides_too_long: "meetings.slides.errors.tooLong",
  slides_panic: "meetings.slides.errors.failed",
  slide_not_found: "meetings.slides.errors.slideNotFound",
  store_failed: "meetings.errors.storeFailed",
  meetings_unavailable: "meetings.errors.meetingsUnavailable",
};

/** Der i18n-Schluessel zu einem Code (auch `code: Detail`), sonst der allgemeine Fehler. */
export const slidesErrorKey = (code: string): string =>
  SLIDE_ERROR_KEYS[code.split(":")[0].trim()] ??
  "meetings.slides.errors.failed";

/** Ende eines Laufs, das der Nutzer als Hinweis (Toast) sieht. */
export type SlidesNotice =
  | { kind: "done"; count: number; added: number }
  | { kind: "stopped"; count: number }
  | { kind: "info" | "error"; errorKey: string };

export const noticeFor = (event: MeetingSlidesEvent): SlidesNotice => {
  switch (event.kind) {
    case "done":
      return { kind: "done", count: event.slides, added: event.added };
    case "stopped":
      return { kind: "stopped", count: event.slides };
    case "skipped":
      return { kind: "info", errorKey: slidesErrorKey(event.code) };
    case "failed":
      return { kind: "error", errorKey: slidesErrorKey(event.code) };
  }
};

// ---------------------------------------------------------------------------
// Import mit "Folien erkennen": die Erkennung folgt, sobald der Import fertig ist
// ---------------------------------------------------------------------------

const PENDING_KEY = "lva.meetings.slidesPending";

/** Besprechungen, bei denen der Import beendet sein soll, bevor die Folien kommen. */
export const readPendingSlides = (): string[] => {
  try {
    const raw = window.localStorage.getItem(PENDING_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed)
      ? parsed.filter((v): v is string => typeof v === "string")
      : [];
  } catch {
    return [];
  }
};

const writePending = (ids: string[]) => {
  try {
    window.localStorage.setItem(PENDING_KEY, JSON.stringify(ids));
  } catch {
    /* nicht merken ist verkraftbar */
  }
};

export const addPendingSlides = (meetingId: string) => {
  const ids = readPendingSlides();
  if (!ids.includes(meetingId)) writePending([...ids, meetingId]);
};

export const removePendingSlides = (meetingId: string) => {
  writePending(readPendingSlides().filter((id) => id !== meetingId));
};
