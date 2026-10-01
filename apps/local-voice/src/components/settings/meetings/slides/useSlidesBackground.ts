import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, events } from "@/bindings";
import {
  addPendingSlides,
  noticeFor,
  readPendingSlides,
  removePendingSlides,
  slidesErrorKey,
} from "@/lib/meetingSlides";

/** Pausen zwischen den Versuchen, wenn die Besprechung nur kurz belegt ist (`job_busy`). */
const RETRY_WAITS_MS = [4_000, 10_000, 30_000, 60_000];

/** Startet die Erkennung mit den Voreinstellungen (Quelle des Imports, 1 Bild je Sekunde). */
export const startSlideDetection = (meetingId: string) =>
  commands.detectMeetingSlides(meetingId, {
    video_path: null,
    sample_interval_s: null,
  });

/**
 * Vom Import vorgemerkt: die Folienerkennung beginnt, sobald die Besprechung fertig
 * ist (Einstellung "Folien erkennen" beim Import eines Videos). Ein Import ist selbst
 * ein Auftrag der Besprechung, der Folienlauf folgt ihm.
 */
export const queueSlideDetection = (meetingId: string) =>
  addPendingSlides(meetingId);

/**
 * Hintergrundarbeit der Folienerkennung, einmal je Aufnahmen-Seite:
 *  - ein Hinweis (Toast) am Ende jedes Laufs, auch wenn die Besprechung nicht
 *    gewaehlt ist (der Lauf gehoert dem Backend, nicht dem Reiter);
 *  - der Start der beim Import vorgemerkten Besprechungen, sobald ihr Import
 *    endet. Belegt nur ein anderer Auftrag die Besprechung (`job_busy`),
 *    wird kurz danach neu versucht; eine Aufnahme oder ein anderer Fehler
 *    beendet das Vormerken mit einem Hinweis.
 */
export function useSlidesBackground() {
  const { t } = useTranslation();
  const tRef = useRef(t);
  tRef.current = t;
  const trying = useRef(new Set<string>());
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  useEffect(() => {
    const timerMap = timers.current;
    let alive = true;

    const attempt = (meetingId: string, round: number) => {
      if (!alive || trying.current.has(meetingId)) return;
      if (!readPendingSlides().includes(meetingId)) return;
      trying.current.add(meetingId);
      void startSlideDetection(meetingId)
        .then((result) => {
          trying.current.delete(meetingId);
          if (result.status === "ok") {
            removePendingSlides(meetingId);
            return;
          }
          const code = result.error;
          const head = code.split(":")[0].trim();
          if (head === "meeting_not_finished") return; // Import laeuft noch: das Ende kommt als Ereignis
          if (head === "job_busy" && round < RETRY_WAITS_MS.length) {
            timerMap.set(
              meetingId,
              setTimeout(() => {
                timerMap.delete(meetingId);
                attempt(meetingId, round + 1);
              }, RETRY_WAITS_MS[round]),
            );
            return;
          }
          removePendingSlides(meetingId);
          toast.error(tRef.current(slidesErrorKey(code)));
        })
        .catch(() => trying.current.delete(meetingId));
    };

    // Vorgemerkt, aber die Seite war zu: jetzt nachholen (laeuft der Import noch,
    // sagt das Backend `meeting_not_finished` und das Ende-Ereignis uebernimmt).
    for (const id of readPendingSlides()) attempt(id, 0);

    const unMeeting = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind === "state") {
        if (payload.status === "failed" || payload.status === "cancelled") {
          removePendingSlides(payload.meeting_id);
        } else if (payload.status === "ready") {
          attempt(payload.meeting_id, 0);
        }
      } else if (payload.kind === "job_ended" && payload.phase !== "slides") {
        attempt(payload.meeting_id, 0);
      }
    });

    const unSlides = events.meetingSlidesEvent.listen((e) => {
      const notice = noticeFor(e.payload);
      removePendingSlides(e.payload.meeting_id);
      switch (notice.kind) {
        case "done":
          toast.success(
            tRef.current("meetings.slides.done", { count: notice.count }),
          );
          break;
        case "stopped":
          toast.info(
            tRef.current("meetings.slides.stopped", { count: notice.count }),
          );
          break;
        case "info":
          toast.info(tRef.current(notice.errorKey));
          break;
        case "error":
          toast.error(tRef.current(notice.errorKey));
          break;
      }
    });

    return () => {
      alive = false;
      for (const timer of timerMap.values()) clearTimeout(timer);
      timerMap.clear();
      void unMeeting.then((f) => f());
      void unSlides.then((f) => f());
    };
  }, []);
}
