import type { StoredSegment } from "@/bindings";

/**
 * Reihenfolge der Anzeige: nach Beginn, bei Gleichstand nach Segmentnummer.
 * Mikrofon und Gegenseite werden getrennt transkribiert und treffen daher nie
 * in Lesereihenfolge ein.
 */
export const bySegmentOrder = (a: StoredSegment, b: StoredSegment): number =>
  a.start_ms - b.start_ms || a.segment_index - b.segment_index;

/**
 * Vereinigt zwei Segmentlisten nach `segment_index` (innerhalb einer
 * Besprechung eindeutig). Bei gleicher Nummer gewinnt `incoming`. So ist das
 * Zusammenführen idempotent: ein Segment, das sowohl aus der Datenbank als auch
 * aus einem Ereignis kommt, steht genau einmal da, und keines geht verloren,
 * gleich in welcher Reihenfolge Ladevorgang und Ereignisse eintreffen.
 *
 * Gibt `existing` unverändert zurück, wenn es nichts hinzuzufügen gibt (spart
 * einen Render).
 */
export const mergeSegments = (
  existing: StoredSegment[],
  incoming: StoredSegment[],
): StoredSegment[] => {
  if (incoming.length === 0) return existing;
  const byIndex = new Map<number, StoredSegment>();
  for (const segment of existing) byIndex.set(segment.segment_index, segment);
  for (const segment of incoming) byIndex.set(segment.segment_index, segment);
  return [...byIndex.values()].sort(bySegmentOrder);
};
