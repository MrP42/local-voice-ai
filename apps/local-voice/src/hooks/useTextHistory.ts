import { useCallback, useEffect, useRef, useState } from "react";

/** Wie lange aufeinanderfolgende Aenderungen als EIN Schritt gelten. */
const COALESCE_MS = 700;
/** Obergrenze je Schluessel — bei 100-KB-Texten sind das ~20 MB, mehr nicht. */
const MAX_STEPS = 200;

interface HistoryEntry {
  past: string[];
  present: string;
  future: string[];
  lastAt: number;
  /** Direkt nach einem Schluesselwechsel: die erste Aenderung ist das Laden
   *  der Seite, kein Bearbeitungsschritt. */
  initializing: boolean;
}

/** Ob `next` aus `prev` durch EINE zusammenhaengende Aenderung von hoechstens
 *  zwei Zeichen hervorgeht (Tippen, Loeschen) -- alles andere (Einfuegen,
 *  Ersetzen, Auto-Tagging, Aufbereiten) ist ein eigener Schritt. */
const isTypingEdit = (prev: string, next: string): boolean => {
  let a = 0;
  const max = Math.min(prev.length, next.length);
  while (a < max && prev[a] === next[a]) a++;
  let b = 0;
  while (b < max - a && prev[prev.length - 1 - b] === next[next.length - 1 - b])
    b++;
  return prev.length - a - b <= 2 && next.length - a - b <= 2;
};

/**
 * Bearbeitungshistorie fuer einen Text, getrennt je `key` (Seite + Reiter).
 * Tippen innerhalb von 700 ms wird zu einem Schritt zusammengefasst — wie in
 * jedem Editor. Rueckgaengig/Wiederherstellen setzen den Text ueber
 * `setValue`; diese Aenderung wird nicht als neuer Schritt gezaehlt.
 *
 * Eigene Historie statt der nativen der Textarea, weil Auto-Tagging,
 * Aufbereiten, Ersetzen und die Skript-Pruefung den Text programmatisch
 * setzen — die native Historie kennt diese Schritte nicht und verliert sie
 * beim Seitenwechsel. Die Historie lebt im Speicher der Sitzung.
 */
export function useTextHistory(
  key: string,
  value: string,
  setValue: (next: string) => void,
) {
  const entries = useRef(new Map<string, HistoryEntry>());
  const skipNext = useRef(false);
  const [, bump] = useState(0);

  const entryFor = (k: string): HistoryEntry => {
    let e = entries.current.get(k);
    if (!e) {
      e = {
        past: [],
        present: value,
        future: [],
        lastAt: 0,
        initializing: true,
      };
      entries.current.set(k, e);
    }
    return e;
  };

  useEffect(() => {
    const e = entryFor(key);
    if (value === e.present) return;
    if (skipNext.current) {
      skipNext.current = false;
      e.present = value;
      bump((n) => n + 1);
      return;
    }
    if (e.initializing) {
      e.initializing = false;
      e.present = value;
      e.lastAt = 0;
      return;
    }
    const now = Date.now();
    // Nur Tippen verschmilzt (ein, zwei Zeichen); Einfuegen, Ersetzen,
    // Auto-Tagging und Aufbereiten sind immer ein eigener Schritt.
    const typing = isTypingEdit(e.present, value);
    if (!typing || now - e.lastAt > COALESCE_MS) {
      e.past.push(e.present);
      if (e.past.length > MAX_STEPS) e.past.shift();
    }
    e.present = value;
    e.future = [];
    e.lastAt = now;
    bump((n) => n + 1);
  }, [key, value]);

  useEffect(() => {
    // Neuer Schluessel: das naechste Setzen ist das Laden, kein Schritt.
    const e = entryFor(key);
    e.initializing = value !== e.present;
  }, [key]);

  const undo = useCallback(() => {
    const e = entryFor(key);
    const prev = e.past.pop();
    if (prev === undefined) return;
    e.future.push(e.present);
    e.lastAt = 0;
    skipNext.current = true;
    setValue(prev);
  }, [key, setValue]);

  const redo = useCallback(() => {
    const e = entryFor(key);
    const next = e.future.pop();
    if (next === undefined) return;
    e.past.push(e.present);
    e.lastAt = 0;
    skipNext.current = true;
    setValue(next);
  }, [key, setValue]);

  /** `n` Schritte zurueck (n > 0) oder vor (n < 0) -- ein einziges
   *  setValue, damit der Editor nicht n-mal neu rendert. */
  const jump = useCallback(
    (n: number) => {
      const e = entryFor(key);
      let current = e.present;
      if (n > 0) {
        for (let i = 0; i < n && e.past.length > 0; i++) {
          e.future.push(current);
          current = e.past.pop() as string;
        }
      } else {
        for (let i = 0; i < -n && e.future.length > 0; i++) {
          e.past.push(current);
          current = e.future.pop() as string;
        }
      }
      if (current === e.present) return;
      e.lastAt = 0;
      skipNext.current = true;
      setValue(current);
    },
    [key, setValue],
  );

  const e = entries.current.get(key);
  return {
    undo,
    redo,
    jump,
    /** Zustaende vor dem jetzigen, neuester zuerst. */
    past: [...(e?.past ?? [])].reverse(),
    /** Zurueckgenommene Zustaende, naechster zuerst. */
    future: [...(e?.future ?? [])].reverse(),
    present: e?.present ?? value,
    canUndo: (e?.past.length ?? 0) > 0,
    canRedo: (e?.future.length ?? 0) > 0,
  };
}
