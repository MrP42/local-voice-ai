import { useEffect, useRef } from "react";
import type { Meeting } from "@/bindings";

/**
 * G1 (#70): der leere Eintrag. Eine Besprechung ohne Audio und ohne Quelle, in
 * die der Nutzer Notizen schreibt und die er spaeter fuellt (Aufnahme, Datei,
 * YouTube-Link). Das Backend kennzeichnet sie mit `source = "empty"`.
 */
export const EMPTY_SOURCE = "empty";

export const isEmptyEntry = (
  meeting: Pick<Meeting, "source"> | null | undefined,
): boolean => meeting?.source === EMPTY_SOURCE;

/**
 * Wunsch "Startdialog der Aufnahme oeffnen" von einer Stelle, die ihn nicht
 * selbst besitzt (die Startflaeche des leeren Eintrags in der Mitte). Der Dialog
 * gehoert der Aufnahmekarte rechts; ist sie gerade nicht eingehaengt (Spalte
 * eingeklappt), bleibt der Wunsch stehen, bis sie da ist.
 */
let pending = false;
const listeners = new Set<() => void>();

export const requestStartDialog = () => {
  pending = true;
  listeners.forEach((listener) => listener());
};

/** Holt den offenen Wunsch ab (und loescht ihn). */
export const takeStartDialogRequest = (): boolean => {
  const was = pending;
  pending = false;
  return was;
};

export const useStartDialogRequest = (onRequest: () => void) => {
  const ref = useRef(onRequest);
  ref.current = onRequest;
  useEffect(() => {
    const consume = () => {
      if (takeStartDialogRequest()) ref.current();
    };
    consume();
    listeners.add(consume);
    return () => {
      listeners.delete(consume);
    };
  }, []);
};
