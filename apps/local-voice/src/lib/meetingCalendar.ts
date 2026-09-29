import type { CalEvent } from "@/bindings";

/**
 * Reine Hilfen fuer die Kalenderanzeige der Aufnahmekarte (M5-P5b). Die
 * Entscheidungen (Erinnerung, welcher Termin zur Aufnahme passt) fallen im
 * Backend (`calendar::reminder`); hier steht nur, was die Oberflaeche zum
 * Darstellen braucht.
 */

/**
 * Verschiedene Teilnehmende eines Termins, Organisator eingerechnet. Gleich sind
 * zwei Eintraege mit derselben E-Mail (ohne Gross-/Kleinschreibung), ohne E-Mail
 * zaehlt der Name - wie `reminder::distinct_attendees`.
 */
export const distinctAttendees = (
  event: Pick<CalEvent, "attendees">,
): number => {
  const seen = new Set<string>();
  for (const a of event.attendees) {
    const key = (a.email?.trim() || a.name?.trim() || "").toLowerCase();
    if (key) seen.add(key);
  }
  return seen.size;
};

/** Ende des lokalen Kalendertags (Mitternacht danach) in ms. */
export const endOfLocalDay = (nowMs: number): number => {
  const d = new Date(nowMs);
  d.setHours(24, 0, 0, 0);
  return d.getTime();
};

/** Stunden bis zum Ende des Tages, aufgerundet, mindestens 1 (fuer `calendar_upcoming`). */
export const hoursUntilEndOfDay = (nowMs: number): number =>
  Math.max(1, Math.ceil((endOfLocalDay(nowMs) - nowMs) / 3_600_000));

/**
 * Die Termine fuer "Naechste Termine (heute)": nicht ganztaegig, nicht abgesagt,
 * noch nicht zu Ende, heute begonnen oder laufend; nach Beginn sortiert.
 */
export const todaysEvents = (events: CalEvent[], nowMs: number): CalEvent[] => {
  const end = endOfLocalDay(nowMs);
  return events
    .filter(
      (e) =>
        !e.all_day && !e.cancelled && e.ends_at > nowMs && e.starts_at < end,
    )
    .sort((a, b) => a.starts_at - b.starts_at || a.key.localeCompare(b.key));
};
