import { useEffect, useRef, useState } from "react";
import { commands, events, type MeetingEvent } from "@/bindings";
import {
  applyMeetingEvent,
  hydrateProgress,
  type ProgressMap,
} from "@/lib/meetingJobs";

/**
 * P8a: die laufenden Verarbeitungen aller Besprechungen (Import, Enddurchlauf,
 * Neu-Transkription, Sprecher, KI-Notizen, Protokoll). Der Zustand liegt im
 * Backend: beim Einhaengen wird er abgefragt, danach halten ihn die Ereignisse
 * aktuell. Eine Ansicht, die neu erscheint (Reiterwechsel, andere Seite), sieht
 * deshalb sofort, dass etwas laeuft.
 */
export function useMeetingProgress(): ProgressMap {
  const [map, setMap] = useState<ProgressMap>({});
  // Aufträge, die seit der Abfrage geendet haben: die Antwort der Abfrage ist
  // dann aelter als das Ende und darf sie nicht wieder hinzufuegen.
  const endedAt = useRef<Record<string, number>>({});

  useEffect(() => {
    let alive = true;
    const requestedAt = Date.now();
    const un = events.meetingEvent.listen((e) => {
      const payload: MeetingEvent = e.payload;
      if (
        payload.kind === "job_ended" ||
        (payload.kind === "state" && payload.status !== "processing")
      ) {
        endedAt.current[payload.meeting_id] = Date.now();
      }
      setMap((prev) => applyMeetingEvent(prev, payload, Date.now()));
    });
    void commands.meetingsProgressList().then((result) => {
      if (!alive || result.status !== "ok") return;
      const skip = new Set(
        Object.entries(endedAt.current)
          .filter(([, at]) => at >= requestedAt)
          .map(([id]) => id),
      );
      // Ein Backend ohne die Abfrage (aeltere Staende, Attrappen) liefert `null`.
      setMap((prev) =>
        hydrateProgress(prev, result.data ?? [], Date.now(), skip),
      );
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);

  return map;
}

/** Die Uhr fuer die Anzeige zwischen zwei Ereignissen: tickt nur, solange `active`. */
export function useNow(active: boolean, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [active, intervalMs]);
  return now;
}

/**
 * Ruft `onEnded`, wenn ein Auftrag dieser Besprechung endet (fertig, gestoppt
 * oder gescheitert), auch wenn die Ansicht beim Start noch nicht offen war.
 * Darauf laedt eine Ansicht ihr Ergebnis neu.
 */
export function useJobEnded(
  meetingId: string,
  onEnded: (payload: Extract<MeetingEvent, { kind: "job_ended" }>) => void,
) {
  const latest = useRef(onEnded);
  latest.current = onEnded;
  useEffect(() => {
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind === "job_ended" && payload.meeting_id === meetingId) {
        latest.current(payload);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [meetingId]);
}
