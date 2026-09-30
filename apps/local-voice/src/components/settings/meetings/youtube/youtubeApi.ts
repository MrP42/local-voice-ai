/**
 * Der offizielle YouTube-Einbett-Player (IFrame Player API), ohne Eingriff:
 * kein Werbeblocker, keine Veraenderung der Wiedergabe oder der Werbung, kein
 * verschachteltes iframe (Entscheidung E1, Developer Policies).
 *
 * Datenschutz: nichts hiervon laeuft, bevor der Nutzer "Video abspielen" gedrueckt
 * oder auf eine Zeitmarke geklickt hat. Das Skript `iframe_api` wird erst dann
 * geladen, der Rahmen zeigt auf `youtube-nocookie.com`.
 */

/** Der Host des Einbett-Rahmens. */
export const EMBED_HOST = "https://www.youtube-nocookie.com";
/** Die IFrame Player API (Skript). */
export const API_SCRIPT = "https://www.youtube.com/iframe_api";
/** So lange darf das Laden der API dauern, bevor es als Fehler gilt. */
export const API_TIMEOUT_MS = 15_000;

/** Der Teil der Player-Schnittstelle, den wir benutzen. */
export interface YtPlayer {
  seekTo(seconds: number, allowSeekAhead: boolean): void;
  playVideo(): void;
  pauseVideo(): void;
  getCurrentTime(): number;
}

export interface YtEvents {
  onReady?: () => void;
  onError?: (event: { data: number }) => void;
}

export interface YtNamespace {
  Player: new (
    element: HTMLIFrameElement,
    options: { events?: YtEvents },
  ) => YtPlayer;
}

declare global {
  interface Window {
    YT?: Partial<YtNamespace>;
    onYouTubeIframeAPIReady?: () => void;
  }
}

let apiPromise: Promise<YtNamespace> | null = null;

const ready = (): YtNamespace | null =>
  typeof window.YT?.Player === "function" ? (window.YT as YtNamespace) : null;

/**
 * Laedt die IFrame Player API genau einmal (geteiltes Versprechen). Scheitert
 * das Laden (keine Verbindung, blockiert) oder dauert es zu lang, wird das
 * Versprechen verworfen: der naechste Aufruf versucht es neu.
 */
export function loadYoutubeApi(
  timeoutMs = API_TIMEOUT_MS,
): Promise<YtNamespace> {
  const existing = ready();
  if (existing) return Promise.resolve(existing);
  if (apiPromise) return apiPromise;

  apiPromise = new Promise<YtNamespace>((resolve, reject) => {
    const script = document.createElement("script");
    let settled = false;
    const previous = window.onYouTubeIframeAPIReady;

    const fail = (reason: string) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      script.remove();
      window.onYouTubeIframeAPIReady = previous;
      apiPromise = null;
      reject(new Error(reason));
    };

    const timer = setTimeout(() => fail("timeout"), timeoutMs);

    window.onYouTubeIframeAPIReady = () => {
      previous?.();
      const api = ready();
      if (!api) {
        fail("api_missing");
        return;
      }
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(api);
    };

    script.src = API_SCRIPT;
    script.async = true;
    script.onerror = () => fail("load_failed");
    document.head.appendChild(script);
  });
  return apiPromise;
}

/** Adresse des Einbett-Rahmens (nocookie, API an, eigener Ursprung fuer den Handschlag). */
export function embedUrl(
  videoId: string,
  startSeconds: number | null,
  origin: string,
): string {
  const params = new URLSearchParams({
    enablejsapi: "1",
    origin,
    rel: "0",
    playsinline: "1",
  });
  if (startSeconds && startSeconds > 0) {
    params.set("start", String(Math.floor(startSeconds)));
  }
  return `${EMBED_HOST}/embed/${encodeURIComponent(videoId)}?${params.toString()}`;
}

/** Adresse der Seite bei YouTube (Knopf "Auf YouTube oeffnen"). */
export const watchUrl = (videoId: string, startSeconds?: number | null) =>
  `https://www.youtube.com/watch?v=${encodeURIComponent(videoId)}${
    startSeconds && startSeconds > 0 ? `&t=${Math.floor(startSeconds)}s` : ""
  }`;
