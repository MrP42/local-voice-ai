import { useEffect, useRef } from "react";

/**
 * Kleine Meldung "Die Besprechungsliste hat sich geändert" für Aktionen, die
 * außerhalb der Liste ausgelöst werden (Löschen oder Verschieben im Detailkopf,
 * Import aus der Bedienspalte). Die Liste hört darauf und lädt neu.
 */
const EVENT = "lva:meetings-changed";

export const notifyMeetingsChanged = () => {
  window.dispatchEvent(new CustomEvent(EVENT));
};

export const useMeetingsChanged = (callback: () => void) => {
  const ref = useRef(callback);
  ref.current = callback;
  useEffect(() => {
    const handler = () => ref.current();
    window.addEventListener(EVENT, handler);
    return () => window.removeEventListener(EVENT, handler);
  }, []);
};

// ---------------------------------------------------------------------------
// Sprung zu einem Segment (Herkunft eines Agent-Schritts, C5)
// ---------------------------------------------------------------------------

/** Eine Stelle im Transkript: die Besprechung und das Segment (`null`: nur die Besprechung). */
export interface OpenSegmentRequest {
  meetingId: string;
  segmentIndex: number | null;
}

const OPEN_EVENT = "lva:open-segment";
/** Der Wunsch bleibt stehen, bis die Besprechungsseite ihn abholt (sie ist evtl. noch nicht da). */
let pendingOpen: OpenSegmentRequest | null = null;

/** Wechselt zu den Aufnahmen und oeffnet dort die Besprechung an dieser Stelle. */
export const requestOpenSegment = (request: OpenSegmentRequest) => {
  pendingOpen = request;
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "meetings" } }),
  );
  window.dispatchEvent(new CustomEvent(OPEN_EVENT));
};

/** Die Besprechungsseite holt einen offenen Wunsch beim Start und bei jedem neuen ab. */
export const useOpenSegment = (callback: (request: OpenSegmentRequest) => void) => {
  const ref = useRef(callback);
  ref.current = callback;
  useEffect(() => {
    const take = () => {
      const request = pendingOpen;
      pendingOpen = null;
      if (request) ref.current(request);
    };
    take();
    window.addEventListener(OPEN_EVENT, take);
    return () => window.removeEventListener(OPEN_EVENT, take);
  }, []);
};
