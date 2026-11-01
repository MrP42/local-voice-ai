import { useSyncExternalStore } from "react";

/**
 * Brücke zwischen Seitenkopf und Hilfe-Menü der Titelleiste: Eine Seite mit
 * Kontexthilfe (`PageShell help=…`) meldet hier ihren Umschalter an, das Menü
 * „Hilfe“ ruft ihn auf. Es gibt höchstens eine sichtbare Seite.
 */
let toggle: (() => void) | null = null;
const listeners = new Set<() => void>();

const emit = () => listeners.forEach((l) => l());

export function registerPageHelp(fn: () => void): () => void {
  toggle = fn;
  emit();
  return () => {
    if (toggle === fn) {
      toggle = null;
      emit();
    }
  };
}

export const togglePageHelp = () => toggle?.();

export function useHasPageHelp(): boolean {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => toggle !== null,
    () => false,
  );
}
