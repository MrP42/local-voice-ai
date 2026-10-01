import { useEffect, useRef, useState } from "react";
import { commands, events } from "@/bindings";

/**
 * Laeuft gerade eine Aufnahme (auch pausiert), und welche Besprechung ist es?
 * Die Aufnahmen-Seite braucht das, um die rechte Spalte nicht zuklappen zu
 * lassen (dort sitzt der Stopp-Knopf) und den Notizblock zu zeigen. Beim
 * Einhaengen fragt der Hook das Backend, danach halten ihn die Ereignisse
 * aktuell.
 */
export function useRecordingActive(): {
  active: boolean;
  meetingId: string | null;
} {
  const [active, setActive] = useState(false);
  const [meetingId, setMeetingId] = useState<string | null>(null);
  // Die Besprechung, die gerade aufgenommen wird: nur ihr Ende beendet die
  // Aufnahme. Ereignisse anderer Besprechungen (ein Import, der waehrend der
  // Aufnahme fertig wird, eine Neu-Transkription) duerfen sie nicht beenden.
  const recordingId = useRef<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands.meetingsRecordingPosition().then((result) => {
      if (!cancelled && result.status === "ok" && result.data) {
        recordingId.current = recordingId.current ?? result.data.meeting_id;
        setActive(true);
        setMeetingId((prev) => prev ?? result.data!.meeting_id);
      }
    });
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind !== "state") return;
      if (payload.status === "recording" || payload.status === "paused") {
        recordingId.current = payload.meeting_id;
        setActive(true);
        if (payload.status === "recording") setMeetingId(payload.meeting_id);
        return;
      }
      if (
        recordingId.current !== null &&
        payload.meeting_id !== recordingId.current
      ) {
        return;
      }
      recordingId.current = null;
      setActive(false);
    });
    return () => {
      cancelled = true;
      void un.then((f) => f());
    };
  }, []);

  return { active, meetingId };
}
