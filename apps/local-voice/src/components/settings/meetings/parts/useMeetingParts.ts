import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type PartsInfo } from "@/bindings";
import { useMeetingsChanged } from "@/lib/meetingsBus";

export const EMPTY_PARTS: PartsInfo = {
  parts: [],
  pending: [],
  append_to: null,
};

/** Zeitmarke wie im Transkript: M:SS, ab einer Stunde H:MM:SS. */
export const formatPartTime = (ms: number): string => {
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = (total % 60).toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${s}` : `${m}:${s}`;
};

/**
 * Die Teile einer Besprechung (zusammengefuehrte Dateien, noch laufende Teile,
 * die angehaengt werden, und fuer einen solchen Teil sein Ziel). Laedt neu bei
 * jedem Zustandswechsel einer Besprechung und bei Aenderungen der Liste: ein
 * Teil wird fertig, die Gruppe wird zusammengefuehrt, ein Teil wird geloest.
 */
export function useMeetingParts(meetingId: string | null) {
  const [info, setInfo] = useState<PartsInfo>(EMPTY_PARTS);
  // Nur die Antwort der juengsten Anfrage gilt (Wechsel der Besprechung).
  const latest = useRef(0);

  const reload = useCallback(async () => {
    const request = ++latest.current;
    if (!meetingId) {
      setInfo(EMPTY_PARTS);
      return;
    }
    const result = await commands.meetingsPartsInfo(meetingId);
    if (request !== latest.current) return;
    setInfo(result.status === "ok" && result.data ? result.data : EMPTY_PARTS);
  }, [meetingId]);

  useEffect(() => {
    setInfo(EMPTY_PARTS);
    void reload();
  }, [reload]);

  useMeetingsChanged(() => void reload());

  useEffect(() => {
    const un = events.meetingEvent.listen((e) => {
      const kind = e.payload.kind;
      if (kind === "state" || kind === "reset") void reload();
    });
    return () => {
      void un.then((f) => f());
    };
  }, [reload]);

  return { info, reload };
}
