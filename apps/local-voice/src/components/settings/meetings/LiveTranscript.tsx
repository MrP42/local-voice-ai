import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { events, type StoredSegment } from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import Badge from "../../ui/Badge";

const formatMmSs = (ms: number) => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

const channelLabelKey = (channel: number) => {
  switch (channel) {
    case 0:
      return "meetings.live.me";
    case 1:
      return "meetings.live.remote";
    default:
      return "meetings.live.mixed";
  }
};

/**
 * Live transcript for the meeting currently being recorded.
 *
 * Segments arrive interleaved across channels (mic vs. system audio each
 * transcribe independently), so they are always re-sorted by `start_ms`
 * before rendering rather than trusted to arrive in reading order.
 */
export const LiveTranscript: React.FC = () => {
  const { t } = useTranslation();
  const [segments, setSegments] = useState<StoredSegment[]>([]);
  const [activeMeetingId, setActiveMeetingId] = useState<string | null>(null);
  // Ref mirror of the active meeting so the (mount-once) listener can filter
  // without going stale: segment events from OTHER meetings — an import
  // running while a recording is live is the everyday case — must never be
  // merged into this transcript (M8 acceptance follow-up 7.1).
  const activeIdRef = useRef<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind === "state") {
        if (payload.status === "recording") {
          activeIdRef.current = payload.meeting_id;
          setActiveMeetingId((prev) =>
            prev === payload.meeting_id ? prev : payload.meeting_id,
          );
        } else if (
          payload.status === "processing" ||
          payload.status === "ready"
        ) {
          // Recording ended — leave the transcript visible until a new
          // recording starts rather than clearing it out from under the user.
        }
      } else if (payload.kind === "segments") {
        if (payload.meeting_id !== activeIdRef.current) return;
        setSegments((prev) => {
          const merged = [...prev, ...payload.appended];
          return merged.sort((a, b) => a.start_ms - b.start_ms);
        });
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // A freshly started recording starts its transcript from empty.
  useEffect(() => {
    if (activeMeetingId) setSegments([]);
  }, [activeMeetingId]);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [segments]);

  if (!activeMeetingId && segments.length === 0) return null;

  // Dieselbe Regel wie in der Detailansicht: Eine Quellenangabe, die auf
  // jeder Zeile gleich lautet, unterscheidet nichts. Sichtbar wird sie
  // erst, sobald Mikrofon und Gegenseite getrennt eintreffen.
  const showChannels = new Set(segments.map((s) => s.channel)).size > 1;

  return (
    <SettingsGroup>
      <div
        ref={listRef}
        className="px-4 py-3 space-y-2 max-h-80 overflow-y-auto"
      >
        {segments.length === 0 ? (
          <p className="text-sm text-text/60">{t("meetings.live.empty")}</p>
        ) : (
          segments.map((segment) => (
            <div
              key={`${segment.channel}-${segment.segment_index}`}
              className="flex gap-2 items-start text-sm"
            >
              <span className="text-xs text-text/40 w-10 shrink-0 pt-0.5">
                {formatMmSs(segment.start_ms)}
              </span>
              {showChannels && (
                <Badge variant="secondary" className="shrink-0">
                  {t(channelLabelKey(segment.channel))}
                </Badge>
              )}
              <p className="text-text/90 break-words">{segment.text}</p>
            </div>
          ))
        )}
      </div>
    </SettingsGroup>
  );
};
