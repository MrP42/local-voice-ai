import { useEffect, useState } from "react";

/**
 * Liefert `value` erst, wenn es `delayMs` lang unveraendert war. Fuer
 * Analysen, die nicht bei jedem Tastendruck laufen muessen (Skript-
 * Pruefung): die Textarea reagiert sofort, die Auswertung folgt kurz danach.
 */
export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const handle = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(handle);
  }, [value, delayMs]);
  return debounced;
}
