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
