import { useSyncExternalStore } from "react";

/**
 * Sperre "Dateien werden kopiert" je Projekt. Sie liegt außerhalb der Seite: eine neu eingehängte
 * Seite (Bereichswechsel und zurück) sieht eine noch laufende Kopie und bietet keine zweite an.
 */
const laufend = new Set<string>();
const hoerer = new Set<() => void>();

const melden = () => hoerer.forEach((h) => h());
const abonnieren = (h: () => void) => {
  hoerer.add(h);
  return () => {
    hoerer.delete(h);
  };
};

/** Nimmt die Sperre; false, wenn das Projekt schon kopiert. */
export const kopierStart = (projektId: string): boolean => {
  if (laufend.has(projektId)) return false;
  laufend.add(projektId);
  melden();
  return true;
};

export const kopierEnde = (projektId: string): void => {
  if (laufend.delete(projektId)) melden();
};

export const useKopiert = (projektId: string | null): boolean =>
  useSyncExternalStore(
    abonnieren,
    () => projektId !== null && laufend.has(projektId),
    () => false,
  );
