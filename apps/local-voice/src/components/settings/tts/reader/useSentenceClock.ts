import { useEffect, useState } from "react";
import {
  clockElapsed,
  clockFraction,
  type SentenceClock,
} from "@/lib/speechTiming";

export { clockElapsed, clockFraction };
export type { SentenceClock };

/**
 * Läuft mit dem Audio mit: liefert den Anteil des Satzes, neu berechnet
 * `hz`-mal je Sekunde. Ohne Uhr 0 und kein Timer.
 */
export function useSentenceClock(clock: SentenceClock | null, hz = 8): number {
  return useClockValue(clock, hz, clockFraction);
}

/** Wie {@link useSentenceClock}, aber die vergangene Zeit (ms bei Tempo 1). */
export function useSentenceElapsed(
  clock: SentenceClock | null,
  hz = 8,
): number {
  return useClockValue(clock, hz, clockElapsed);
}

function useClockValue(
  clock: SentenceClock | null,
  hz: number,
  read: (clock: SentenceClock, now: number) => number,
): number {
  const [value, setValue] = useState(0);
  useEffect(() => {
    if (!clock || clock.ms <= 0) {
      setValue(0);
      return;
    }
    const tick = () => setValue(read(clock, performance.now()));
    tick();
    const id = window.setInterval(tick, 1000 / hz);
    return () => window.clearInterval(id);
  }, [clock, hz, read]);
  return value;
}
