import type { TFunction } from "i18next";

/**
 * Fehlercodes der YouTube-Kommandos (`youtube_*`). Ein Code kann einen Zusatz
 * tragen (`youtube_network: <Text>`, `youtube_http: 500`); der Teil vor dem
 * ersten Doppelpunkt waehlt den Text, der Rest steht als `{{detail}}` darin.
 */
const KNOWN = new Set([
  "youtube_empty",
  "youtube_too_long",
  "youtube_invalid_link",
  "youtube_not_youtube",
  "youtube_playlist",
  "youtube_channel",
  "youtube_disabled",
  "youtube_busy",
  "youtube_project_not_found",
  "youtube_unavailable",
  "youtube_rate_limited",
  "youtube_timeout",
  "youtube_network",
  "youtube_http",
  "youtube_bad_response",
  "youtube_store_failed",
  "youtube_path_invalid",
  "youtube_private_off",
  "youtube_tool_missing",
  "youtube_tool_outdated",
  "youtube_tool_failed",
  "youtube_tool_start",
  "youtube_no_subtitles",
  "youtube_cancelled",
  // G1 (#70): der Link sollte einen leeren Eintrag fuellen, der es nicht mehr ist.
  "target_not_empty",
  "meeting_not_found",
]);

export const youtubeErrorCode = (raw: string): string =>
  raw.split(":")[0].trim();

/** Meldung fuer einen Fehlercode; Unbekanntes erscheint unveraendert. */
export const translateYoutubeError = (raw: string, t: TFunction): string => {
  const code = youtubeErrorCode(raw);
  if (!KNOWN.has(code)) return raw;
  const detail = raw.includes(":")
    ? raw.slice(raw.indexOf(":") + 1).trim()
    : "";
  return t(`meetings.youtube.errors.${code}`, { detail }).trim();
};
