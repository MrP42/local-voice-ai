import type { QueueSnapshot, WaitReason } from "@/bindings";

/**
 * U7: die Import-Warteschlange aus Sicht der Oberflaeche. Reine Funktionen ohne
 * React und Tauri: wo steht eine Besprechung, warum wartet sie.
 */

/** Platz einer wartenden Besprechung (ab 1) und warum die Schlange steht. */
export interface QueuePlace {
  position: number;
  /** Zahl der wartenden Besprechungen. */
  total: number;
  /** `null`: die Datei beginnt gleich. */
  reason: WaitReason | null;
}

/** Steht die Besprechung in der Warteschlange und wartet? */
export const queuePlace = (
  snapshot: QueueSnapshot | null | undefined,
  meetingId: string,
): QueuePlace | null => {
  if (!snapshot) return null;
  const index = snapshot.waiting.indexOf(meetingId);
  if (index < 0) return null;
  return {
    position: index + 1,
    total: snapshot.waiting.length,
    reason: snapshot.blocked,
  };
};

/** Wurde dieser laufende Import wegen einer Aufnahme angehalten? */
export const heldForRecording = (
  snapshot: QueueSnapshot | null | undefined,
  meetingId: string,
): boolean => snapshot?.held.includes(meetingId) ?? false;

/** Wie viele Dateien warten oder laufen (fuer eine knappe Anzeige)? */
export const queueSize = (
  snapshot: QueueSnapshot | null | undefined,
): number => (snapshot ? snapshot.waiting.length + snapshot.running.length : 0);

/** Fehlercodes der Warteschlangen-Befehle als i18n-Schluessel. */
export const QUEUE_ERROR_KEYS: Record<string, string> = {
  not_in_queue: "meetings.queue.errors.notInQueue",
  not_queued: "meetings.queue.errors.notQueued",
};
