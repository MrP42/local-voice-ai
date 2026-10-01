import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  commands,
  events,
  type MeetingSlide,
  type MeetingSlidesEvent,
} from "@/bindings";
import { slideFilePath } from "@/lib/meetingSlides";

export interface MeetingSlidesState {
  /** Alle Folien der Besprechung, auch ausgeblendete (nach Nummer). */
  slides: MeetingSlide[];
  /** Erster Abruf beendet? Davor zeigt die Oberflaeche nichts als "keine Folien". */
  loaded: boolean;
  /** Bild-URL einer Datei der Folie (`image_path` oder `thumb_path`), oder `null` ohne Ordner. */
  imageUrl: (relative: string | null) => string | null;
  /** Folien neu lesen. */
  reload: () => Promise<void>;
  /** Blendet eine Folie aus oder ein; der Zustand springt sofort um, ein Fehler nimmt ihn zurueck. */
  setHidden: (slideId: string, hidden: boolean) => Promise<string | null>;
}

/**
 * Die Folien einer Besprechung (D1: `list_meeting_slides`). Sie werden beim
 * Wechsel der Besprechung gelesen und neu, sobald ein Folienlauf dieser Besprechung
 * endet (`MeetingSlidesEvent`). `onEnded` kommt bei jedem Ende dieser Besprechung,
 * nachdem die Folien neu gelesen sind. Hinweise (Toasts) gehoeren nicht hierher:
 * sie kommen einmal je Lauf aus `useSlidesBackground`.
 */
export function useMeetingSlides(
  meetingId: string,
  onEnded?: (event: MeetingSlidesEvent, slides: MeetingSlide[]) => void,
): MeetingSlidesState {
  const [slides, setSlides] = useState<MeetingSlide[]>([]);
  const [dir, setDir] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const idRef = useRef(meetingId);
  idRef.current = meetingId;
  const endedRef = useRef(onEnded);
  endedRef.current = onEnded;

  const load = useCallback(async (): Promise<MeetingSlide[]> => {
    const id = idRef.current;
    const result = await commands.listMeetingSlides(id);
    // Eine Antwort fuer eine Besprechung, die inzwischen nicht mehr gewaehlt ist, zaehlt nicht.
    if (id !== idRef.current) return [];
    // Ein Backend ohne den Befehl (Attrappen) liefert `null`.
    const list = result.status === "ok" ? (result.data ?? []) : [];
    setSlides(list);
    setLoaded(true);
    return list;
  }, []);

  useEffect(() => {
    setSlides([]);
    setDir(null);
    setLoaded(false);
    let cancelled = false;
    void load();
    void commands.meetingSlidesDir(meetingId).then((result) => {
      if (!cancelled && result.status === "ok" && result.data)
        setDir(result.data);
    });
    return () => {
      cancelled = true;
    };
  }, [meetingId, load]);

  useEffect(() => {
    const un = events.meetingSlidesEvent.listen((e) => {
      if (e.payload.meeting_id !== idRef.current) return;
      const payload = e.payload;
      void load().then((list) => endedRef.current?.(payload, list));
    });
    return () => {
      void un.then((f) => f());
    };
  }, [load]);

  const imageUrl = useCallback(
    (relative: string | null) =>
      dir && relative
        ? convertFileSrc(slideFilePath(dir, relative), "asset")
        : null,
    [dir],
  );

  const setHidden = useCallback(async (slideId: string, hidden: boolean) => {
    setSlides((prev) =>
      prev.map((s) => (s.id === slideId ? { ...s, hidden } : s)),
    );
    const result = await commands.setMeetingSlideHidden(slideId, hidden);
    if (result.status === "ok") return null;
    setSlides((prev) =>
      prev.map((s) => (s.id === slideId ? { ...s, hidden: !hidden } : s)),
    );
    return result.error;
  }, []);

  return {
    slides,
    loaded,
    imageUrl,
    reload: async () => void (await load()),
    setHidden,
  };
}
