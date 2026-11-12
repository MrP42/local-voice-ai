/**
 * lva#522: Zeitfelder und Bereich der Aufnahme für „Bereich transkribieren" und
 * „Aufnahme zuschneiden". Reine Funktionen, ohne React und ohne Tauri.
 */

/** Auflösung von Regler und Feldern: ganze Sekunden. */
export const RANGE_STEP_MS = 1000;

export interface MsRange {
  start_ms: number;
  end_ms: number;
}

/**
 * Zeit als Text: `m:ss` unter einer Stunde, sonst `h:mm:ss` (ganze Sekunden,
 * abgerundet).
 */
export const formatRangeTime = (ms: number): string => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
  return `${m}:${ss}`;
};

/**
 * Tolerant lesen: `h:mm:ss`, `m:ss`, reine Sekunden (`90`), Komma oder Punkt als
 * Dezimaltrenner bei den Sekunden (`1:02,5`), Leerraum drumherum. Minuten und
 * Sekunden über 59 werden nicht abgelehnt (`90:00` sind 90 Minuten). Ergebnis in
 * ganzen Sekunden (ms); Nachkommastellen werden so gerundet, dass der Bereich
 * nichts Gewähltes verliert: „Von“ rundet ab (`floor`, wie die Anzeige), „Bis“ auf
 * (`ceil`). `null` bei allem anderen (leer, Buchstaben, negativ, mehr als drei Teile).
 */
export type RangeRounding = "floor" | "ceil";

export const parseRangeTime = (
  text: string,
  rounding: RangeRounding = "floor",
): number | null => {
  const value = text.trim().replace(",", ".");
  if (!value) return null;
  const parts = value.split(":");
  if (parts.length > 3) return null;
  const numbers: number[] = [];
  for (const [index, part] of parts.entries()) {
    const last = index === parts.length - 1;
    // Nur die Sekunden dürfen Nachkommastellen haben.
    const pattern = last ? /^\d+(\.\d+)?$/ : /^\d+$/;
    if (!pattern.test(part.trim())) return null;
    numbers.push(Number(part));
  }
  let seconds = 0;
  for (const n of numbers) seconds = seconds * 60 + n;
  if (!Number.isFinite(seconds)) return null;
  return (
    (rounding === "ceil" ? Math.ceil(seconds) : Math.floor(seconds)) * 1000
  );
};

/** Auf `[0, durationMs]` klemmen. */
const clamp = (ms: number, durationMs: number): number =>
  Math.min(Math.max(0, ms), Math.max(0, durationMs));

/**
 * Bereich gültig machen: beide Grenzen in `[0, durationMs]`, mindestens eine
 * Sekunde dazwischen. `moved` sagt, welche Grenze der Nutzer bewegt hat; die
 * andere bleibt stehen und die bewegte stößt an sie.
 */
export const normalizeRange = (
  range: MsRange,
  durationMs: number,
  moved: "start" | "end",
): MsRange => {
  const gap = Math.min(RANGE_STEP_MS, Math.max(1, durationMs));
  let start = clamp(range.start_ms, durationMs);
  let end = clamp(range.end_ms, durationMs);
  if (end - start < gap) {
    if (moved === "start") start = Math.max(0, end - gap);
    else end = Math.min(durationMs, start + gap);
    // Am Rand der Aufnahme: die andere Grenze rückt mit.
    if (end - start < gap) {
      if (moved === "start") end = Math.min(durationMs, start + gap);
      else start = Math.max(0, end - gap);
    }
  }
  return { start_ms: start, end_ms: end };
};

/** Der Bereich ist die ganze Aufnahme (nichts zu schneiden). */
export const coversAll = (range: MsRange, durationMs: number): boolean =>
  range.start_ms <= 0 && range.end_ms >= durationMs;

/** Länge des Bereichs. */
export const rangeLength = (range: MsRange): number =>
  Math.max(0, range.end_ms - range.start_ms);

/**
 * Standardbereich für die Dauer: die ganze Aufnahme.
 */
export const fullRange = (durationMs: number): MsRange => ({
  start_ms: 0,
  end_ms: Math.max(0, durationMs),
});

/**
 * Dauer der Aufnahme für die Auswahl: die gespeicherte Dauer, sonst `null`
 * (dann gibt es keinen Bereichswähler).
 */
export const recordingDurationMs = (
  durationMs: number | null | undefined,
): number | null =>
  typeof durationMs === "number" && durationMs > 0 ? durationMs : null;
