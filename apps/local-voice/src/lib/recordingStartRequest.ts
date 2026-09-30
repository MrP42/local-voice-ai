import type { CalEvent } from "@/bindings";

/**
 * Wunsch "Aufnahme zu diesem Termin starten" von einer Stelle, die den
 * Startdialog nicht selbst besitzt (Abschnitt "Als Naechstes" in der
 * Projekte-Spalte). Der Startdialog gehoert der Aufnahmekarte; sie holt den
 * Wunsch ab, sobald sie da ist. Ist die rechte Spalte gerade eingeklappt (die
 * Karte also nicht eingehaengt), bleibt der Wunsch stehen, bis die Spalte
 * wieder offen ist.
 */
export interface StartRequest {
  event: CalEvent;
}

let pending: StartRequest | null = null;
const listeners = new Set<() => void>();

export const requestRecordingStart = (event: CalEvent) => {
  pending = { event };
  listeners.forEach((listener) => listener());
};

/** Holt den offenen Wunsch ab (und loescht ihn). */
export const takeStartRequest = (): StartRequest | null => {
  const request = pending;
  pending = null;
  return request;
};

export const hasStartRequest = (): boolean => pending !== null;

export const subscribeStartRequest = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
