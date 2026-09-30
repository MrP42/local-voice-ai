import type { Page } from "@playwright/test";

/**
 * Attrappe der YouTube-Quelle (A2, #66) fuer `youtube-source.spec.ts`. Sie
 * legt sich ueber eine vorhandene Tauri-Attrappe (`recLayoutMock` oder
 * `calendarMock`) und uebernimmt nur die neuen Kommandos.
 *
 * - Die Link-Pruefung spiegelt die Fehlercodes des Backends (die echte Logik
 *   prueft `cargo test --lib youtube::link`).
 * - Ein falscher YouTube-Player (`window.YT`) schreibt jeden Aufruf mit
 *   (`__yt.calls`) und meldet sich wie der echte per `onReady`.
 * - Jeder Netzzugriff auf YouTube-Hosts wird abgefangen und gezaehlt
 *   (`youtubeRequests`): vor einem Klick auf "Video abspielen" darf es keinen geben.
 */

export const YT_ID = "dQw4w9WgXcQ";
export const YT_TITLE = "Lastgang verstehen: Spitzen glätten";
export const YT_CHANNEL = "Wolff Applied AI";
/** Eine Besprechung mit YouTube-Quelle, die es schon vor dem Test gibt. */
export const YT_MEETING_ID = "y1";

export interface YoutubeMockOptions {
  /** Falscher YouTube-Player statt des echten Skripts (Standard: an). */
  fakePlayer?: boolean;
  /** Besprechung `y1` mit YouTube-Quelle ist schon da (Standard: an). */
  existing?: boolean;
}

export const installYoutubeMock = async (
  page: Page,
  options: YoutubeMockOptions = {},
) => {
  const requests: string[] = [];
  await page.route(
    /^https:\/\/([\w-]+\.)*(youtube|youtube-nocookie|ytimg)\.com\//,
    (route) => {
      requests.push(route.request().url());
      return route.fulfill({
        status: 200,
        contentType: "text/html",
        body: "<!doctype html><title>stub</title>",
      });
    },
  );

  await page.addInitScript(
    ({ fakePlayer, existing, id, title, channel, meetingId }) => {
      const w = window as any;
      w.__yt = { players: [], calls: [] };
      if (fakePlayer) {
        w.YT = {
          Player: class {
            frame: HTMLIFrameElement;
            opts: { events?: Record<string, (e?: unknown) => void> };
            constructor(
              frame: HTMLIFrameElement,
              opts: { events?: Record<string, (e?: unknown) => void> },
            ) {
              this.frame = frame;
              this.opts = opts;
              w.__yt.players.push(this);
              setTimeout(() => opts.events?.onReady?.(), 10);
            }
            seekTo(seconds: number, allowSeekAhead: boolean) {
              w.__yt.calls.push(["seekTo", seconds, allowSeekAhead]);
            }
            playVideo() {
              w.__yt.calls.push(["playVideo"]);
            }
            pauseVideo() {
              w.__yt.calls.push(["pauseVideo"]);
            }
            getCurrentTime() {
              return 0;
            }
          },
        };
      }

      const source = (videoId: string, startS: number | null) => ({
        video_id: videoId,
        url: `https://www.youtube.com/watch?v=${videoId}`,
        title,
        channel,
        channel_url: "https://www.youtube.com/@wolffappliedai",
        thumbnail_url: null,
        start_s: startS,
      });
      w.__ytSources = {};
      w.__toolStatus = {
        found: true,
        path: "C:\\Tools\\yt-dlp.exe",
        version: "2026.09.01",
        source: "path",
        error: null,
      };
      const base = {
        status: "ready",
        source: "youtube",
        started_at: null,
        ended_at: null,
        language: null,
        mic_audio_path: null,
        system_audio_path: null,
        duration_ms: null,
        consent_confirmed_at: null,
        audio_retention_until: null,
        source_path: null,
        deleted_at: null,
      };
      if (existing && Array.isArray(w.__meetings)) {
        w.__meetings = [
          { ...base, id: meetingId, title, created_at: 1_790_100_000 },
          ...w.__meetings,
        ];
        w.__ytSources[meetingId] = source(id, null);
      }

      /** Spiegel von `link::normalize_link`: dieselben Fehlercodes. */
      const normalize = (raw: string) => {
        const text = String(raw).trim();
        if (!text) throw "youtube_empty";
        if (/\s/.test(text)) throw "youtube_invalid_link";
        let url: URL;
        try {
          url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `https://${text}`);
        } catch {
          throw "youtube_invalid_link";
        }
        const host = url.hostname.toLowerCase();
        const hosts = [
          "youtube.com",
          "www.youtube.com",
          "m.youtube.com",
          "music.youtube.com",
          "youtube-nocookie.com",
          "www.youtube-nocookie.com",
        ];
        const seg = url.pathname.split("/").filter(Boolean);
        let videoId: string | null | undefined;
        if (host === "youtu.be") videoId = seg[0];
        else if (hosts.includes(host)) {
          const list = url.searchParams.get("list");
          if (seg[0] === "playlist" || (seg[0] === "watch" && !url.searchParams.get("v") && list)) {
            throw "youtube_playlist";
          }
          if (seg[0]?.startsWith("@") || ["channel", "c", "user"].includes(seg[0])) {
            throw "youtube_channel";
          }
          videoId =
            seg[0] === "watch"
              ? url.searchParams.get("v")
              : ["shorts", "embed", "live", "v"].includes(seg[0])
                ? seg[1]
                : null;
        } else throw "youtube_not_youtube";
        if (!videoId || !/^[A-Za-z0-9_-]{11}$/.test(videoId)) {
          throw "youtube_invalid_link";
        }
        const t = url.searchParams.get("t") ?? url.searchParams.get("start");
        let startS: number | null = null;
        if (t) {
          const m = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s?)?$/i.exec(t);
          if (m) {
            startS = Number(m[1] ?? 0) * 3600 + Number(m[2] ?? 0) * 60 + Number(m[3] ?? 0);
          }
        }
        return { video_id: videoId, url: `https://www.youtube.com/watch?v=${videoId}`, start_s: startS || null };
      };

      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        switch (cmd) {
          case "youtube_normalize_link":
            w.__calls.push({ cmd, args });
            return normalize(args.url as string);
          case "youtube_add_source": {
            w.__calls.push({ cmd, args });
            const link = normalize(args.url as string);
            if (link.video_id === "unavailable") throw "youtube_unavailable";
            const meeting = {
              ...base,
              id: "y-neu",
              title,
              created_at: 1_790_200_000,
            };
            w.__meetings = [meeting, ...(w.__meetings ?? [])];
            w.__ytSources["y-neu"] = source(link.video_id, link.start_s);
            if (args.projectId) w.__folderMap["y-neu"] = [args.projectId];
            return meeting;
          }
          case "youtube_source_get":
            w.__calls.push({ cmd, args });
            return w.__ytSources[args.meetingId as string] ?? null;
          case "youtube_tool_detect":
            w.__calls.push({ cmd, args });
            return w.__toolStatus;
          case "change_meeting_youtube_private_setting":
            w.__calls.push({ cmd, args });
            w.__settings.meeting_youtube_private = args.enabled;
            return null;
          case "change_meeting_youtube_tool_path_setting":
            w.__calls.push({ cmd, args });
            w.__settings.meeting_youtube_tool_path = args.path;
            return null;
        }
        return original(cmd, args);
      };
    },
    {
      fakePlayer: options.fakePlayer ?? true,
      existing: options.existing ?? true,
      id: YT_ID,
      title: YT_TITLE,
      channel: YT_CHANNEL,
      meetingId: YT_MEETING_ID,
    },
  );
  return { youtubeRequests: requests };
};

/** Aufrufe des falschen YouTube-Players in Reihenfolge. */
export const playerCalls = (page: Page): Promise<unknown[][]> =>
  page.evaluate(() => (window as any).__yt.calls as unknown[][]);

export const playerCount = (page: Page): Promise<number> =>
  page.evaluate(() => (window as any).__yt.players.length as number);

/** Text in die Seite "einfuegen" wie Strg+V; liefert, ob das Ereignis abgefangen wurde. */
export const pasteText = (page: Page, text: string): Promise<boolean> =>
  page.evaluate((value) => {
    const data = new DataTransfer();
    data.setData("text/plain", value);
    const event = new ClipboardEvent("paste", {
      clipboardData: data,
      bubbles: true,
      cancelable: true,
    });
    document.body.dispatchEvent(event);
    return event.defaultPrevented;
  }, text);
