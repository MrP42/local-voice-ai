import { chatErrorCode } from "@/lib/meetingChat";

/**
 * Reine Hilfen fuer Personen und Brief (M5-P5d/P5e). Entscheidungen (Abgleich,
 * Zusammenfuehren, Brief-Zuschnitt) fallen im Backend (`managers/people`).
 */

/** Fehlercodes der Personen-Commands (Backend) und ihr i18n-Schluessel. */
const ERROR_KEYS: Record<string, string> = {
  person_not_found: "meetings.people.errors.notFound",
  person_name_invalid: "meetings.people.errors.nameInvalid",
  person_email_invalid: "meetings.people.errors.emailInvalid",
  person_email_taken: "meetings.people.errors.emailTaken",
  person_merge_invalid: "meetings.people.errors.mergeInvalid",
};

/** Schluessel des Fehlertextes; unbekannte Codes ergeben den allgemeinen. */
export const peopleErrorKey = (error: unknown): string =>
  ERROR_KEYS[chatErrorCode(error)] ?? "meetings.people.errors.failed";

/** Anfangsbuchstaben eines Namens (hoechstens zwei) fuer die Chip-Markierung. */
export const initials = (name: string): string =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => Array.from(part)[0]?.toUpperCase() ?? "")
    .join("");

/**
 * Anzeigereihenfolge der Teilnehmenden: wie vom Backend geliefert (Organisator
 * zuerst), nur "ich" rueckt ans Ende, damit die anderen vorn stehen.
 */
export const orderParticipants = <T extends { is_self: boolean }>(
  list: T[],
): T[] => [...list.filter((p) => !p.is_self), ...list.filter((p) => p.is_self)];
