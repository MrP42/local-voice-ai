import { useEffect, useRef } from "react";

/**
 * Meldung "Dialog YouTube-Link oeffnen": der Knopf in der Aufnahmezeile und das
 * Einfuegen per Strg+V loesen sie aus, der Dialog (`YoutubeLinkHost`) sitzt an
 * einer Stelle der Seite und hoert darauf.
 */
const EVENT = "lva:youtube-link-open";

export interface YoutubeLinkRequest {
  /** Vorbelegter Link (aus der Zwischenablage). */
  url?: string;
}

export const openYoutubeLinkDialog = (url?: string) => {
  window.dispatchEvent(
    new CustomEvent<YoutubeLinkRequest>(EVENT, { detail: { url } }),
  );
};

export const useYoutubeLinkOpen = (
  callback: (request: YoutubeLinkRequest) => void,
) => {
  const ref = useRef(callback);
  ref.current = callback;
  useEffect(() => {
    const handler = (event: Event) =>
      ref.current((event as CustomEvent<YoutubeLinkRequest>).detail ?? {});
    window.addEventListener(EVENT, handler);
    return () => window.removeEventListener(EVENT, handler);
  }, []);
};

/**
 * Sieht eingefuegter Text nach einem YouTube-Link aus? Nur ein Vorabtest fuer
 * Strg+V (die Pruefung macht das Backend): der Text BEGINNT mit einem
 * YouTube-Host, sonst wird nichts abgefangen.
 */
const LIKELY =
  /^\s*(?:https?:\/\/)?(?:[\w-]+\.)*(?:youtube\.com|youtu\.be|youtube-nocookie\.com)(?:[/?#:]|\s*$)/i;

export const looksLikeYoutubeLink = (text: string): boolean =>
  LIKELY.test(text);
