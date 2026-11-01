import { useEffect, useState } from "react";

/**
 * Wann der gerade hörbare Satz begann und wie lang er ist (ms bei Tempo 1).
 * Kommt mit `tts-current-sentence`; null ohne laufende Wiedergabe.
 */
export interface SentenceClock {
  index: number | null;
  /** `performance.now()` beim Start des Satzes. */
  at: number;
  ms: number;
  speed: number;
}

/** Anteil (0..1) des laufenden Satzes, der schon zu hören war. */
export function clockFraction(clock: SentenceClock, now: number): number {
  const length = clock.ms / Math.max(0.25, clock.speed);
  if (length <= 0) return 0;
  return Math.max(0, Math.min(1, (now - clock.at) / length));
}

/**
 * Läuft mit dem Audio mit: liefert den Anteil des Satzes, neu berechnet
 * `hz`-mal je Sekunde. Ohne Uhr 0 und kein Timer.
 */
export function useSentenceClock(clock: SentenceClock | null, hz = 8): number {
  const [fraction, setFraction] = useState(0);
  useEffect(() => {
    if (!clock || clock.ms <= 0) {
      setFraction(0);
      return;
    }
    const tick = () => setFraction(clockFraction(clock, performance.now()));
    tick();
    const id = window.setInterval(tick, 1000 / hz);
    return () => window.clearInterval(id);
  }, [clock, hz]);
  return fraction;
}
