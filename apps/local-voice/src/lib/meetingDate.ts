/**
 * Datum und Uhrzeit einer Besprechung, einheitlich in Kopf und Liste (G4, #70):
 * immer mit Jahr, Sprache und Reihenfolge ueber `Intl` (de: "Mo 30.09.2026,
 * 22:54"; Liste kompakt "30.09.2026 · 22:54").
 */

export type MeetingDateStyle = "full" | "compact";

const part = (
  parts: Intl.DateTimeFormatPart[],
  type: Intl.DateTimeFormatPartTypes,
) => parts.find((p) => p.type === type)?.value ?? "";

/**
 * `full`: Wochentag, Datum, Uhrzeit ("Mo 30.09.2026, 22:54").
 * `compact`: Datum und Uhrzeit ohne Wochentag, durch einen Mittelpunkt getrennt
 * ("30.09.2026 · 22:54"). Datum stets zweistellig mit vierstelligem Jahr.
 */
export const formatMeetingDate = (
  date: Date,
  language: string,
  style: MeetingDateStyle = "full",
): string => {
  const day = new Intl.DateTimeFormat(language, {
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
  }).format(date);
  const time = new Intl.DateTimeFormat(language, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
  if (style === "compact") return `${day} · ${time}`;
  const weekday = part(
    new Intl.DateTimeFormat(language, { weekday: "short" }).formatToParts(date),
    "weekday",
  ).replace(/\.$/, "");
  return `${weekday} ${day}, ${time}`;
};

/** Unix-Sekunden (wie `started_at`) in derselben Form. */
export const formatMeetingTimestamp = (
  seconds: number,
  language: string,
  style: MeetingDateStyle = "full",
): string => formatMeetingDate(new Date(seconds * 1000), language, style);
