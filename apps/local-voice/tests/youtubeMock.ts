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
 * - #537 „Beim Abspielen mitschreiben“: `empty` macht `y1` zu einem Eintrag ohne
 *   Transkript, `meetings_start_playback_capture` spiegelt die Regeln des Backends
 *   (Einwilligung, nur YouTube ohne Aufnahme, danach Status `recording`); die Logik
 *   selbst pruefen die Rust-Tests (`cargo test --lib meetings::playback recorder::`).
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
  /** `y1` hat noch kein Transkript (Standard: aus; sonst die Saetze der Rec-Attrappe). */
  empty?: boolean;
  /** Der Schalter „privat“ (yt-dlp) steht an (Standard: aus). */
  privateOn?: boolean;
  /** Fehlercode, mit dem `meetings_start_playback_capture` scheitert (Standard: keiner). */
  startError?: string;
  /**
   * lva#538: `meetings_start_playback_capture` haengt (der Treiber antwortet nicht),
   * bis `meetings_cancel_start` mit DERSELBEN Vorgangs-Id ihn abbricht; der Start meldet
   * dann `start_cancelled`, der Abbruch kehrt mit `null` zurueck (wie `Recorder::cancel_start`).
   */
  startHold?: boolean;
  /**
   * lva#538: der ERSTE `meetings_cancel_start` scheitert mit diesem Code (z. B. `start_busy`:
   * der Start haengt noch in einem Treiber); der zweite Versuch gelingt.
   */
  cancelErrorOnce?: string;
  /**
   * lva#538, Blocker F: der Abbruch setzt den Eintrag zurueck (wie der Rollback des Backends:
   * die gespeicherten Saetze sind danach leer) und meldet dies mit `reset` und `state ready`.
   * Ohne diese Option kommt KEIN Ereignis: nur ein Neuladen der Saetze zeigt den Rollback.
   */
  rollbackEmits?: boolean;
  /**
   * lva#538: der haengende Start laeuft durch, sobald der Abbruch ankommt (der Abbruch kam
   * erst nach der Veroeffentlichung an; das Backend beendet die Aufnahme und setzt zurueck).
   */
  holdFinishesOnCancel?: boolean;
  /** `y1` hat schon einen eigenen Mitschnitt (Audiopfad), samt Transkript. */
  ownAudio?: boolean;
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
    ({
      fakePlayer,
      existing,
      empty,
      privateOn,
      startError,
      startHold,
      cancelErrorOnce,
      rollbackEmits,
      holdFinishesOnCancel,
      ownAudio,
      id,
      title,
      channel,
      meetingId,
    }) => {
      const w = window as any;
      w.__yt = { players: [], calls: [] };
      // #537: ein YouTube-Eintrag ohne Transkript; Saetze kommen erst beim Mitschreiben.
      w.__ytEmpty = empty;
      w.__ytSegments = [];
      w.__playbackError = startError ?? null;
      // lva#538: haengender Start; `w.__heldStart.reject` bricht ihn ab.
      w.__startHold = startHold;
      w.__heldStart = null;
      w.__cancelErrorOnce = cancelErrorOnce ?? null;
      w.__rollbackEmits = rollbackEmits;
      w.__holdFinishesOnCancel = holdFinishesOnCancel;
      // lva#538 (Runde 4): zurueckgehaltene Ladeantworten (`holdLoads`, `releaseHeldLoads`).
      w.__holdLoads = false;
      w.__heldLoads = [];
      if (privateOn) w.__settings.meeting_youtube_private = true;
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
            getDuration() {
              return 1234.5;
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
          {
            ...base,
            id: meetingId,
            title,
            created_at: 1_790_100_000,
            ...(ownAudio
              ? { system_audio_path: "C:/Users/patrick/Aufnahmen/y1-system.wav" }
              : {}),
          },
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
          case "meetings_get_segments":
            w.__calls.push({ cmd, args });
            if (w.__ytEmpty && args.meetingId === meetingId) {
              if (w.__holdLoads) {
                // Die Antwort traegt den Stand vom ZEITPUNKT DER ANFRAGE und kommt erst mit
                // `releaseHeldLoads` (eine langsame Antwort des Backends).
                const snapshot = JSON.parse(JSON.stringify(w.__ytSegments));
                return new Promise((resolve) => {
                  w.__heldLoads.push(() => resolve(snapshot));
                });
              }
              return w.__ytSegments;
            }
            break;
          case "meetings_start_playback_capture": {
            // Spiegel von `Recorder::start_playback_capture`: Einwilligung zuerst,
            // dann nur ein YouTube-Eintrag ohne Aufnahme; sonst bleibt alles wie es war.
            w.__calls.push({ cmd, args });
            if (!args.consentConfirmed) throw "consent_required";
            if (w.__playbackError) throw w.__playbackError;
            const target = (w.__meetings as any[]).find(
              (m) => m.id === args.meetingId,
            );
            if (!target) throw "meeting_not_found";
            if (target.source !== "youtube" || target.status !== "ready") {
              throw "target_not_empty";
            }
            if (w.__startHold) {
              // Der Treiber haengt: der Start endet erst mit dem Abbruch des Nutzers
              // (`meetings_cancel_start` mit derselben Vorgangs-Id).
              return new Promise((_resolve, reject) => {
                w.__heldStart = {
                  reject,
                  resolve: _resolve,
                  opId: args.opId ?? null,
                  meeting: target,
                };
              });
            }
            const started = {
              ...target,
              status: "recording",
              consent_confirmed_at: 1_790_300_000,
            };
            w.__meetings = (w.__meetings as any[]).map((m) =>
              m.id === started.id ? started : m,
            );
            w.__recording = true;
            w.__position = { meeting_id: started.id, position_ms: 0 };
            w.__emit("meeting-event", {
              kind: "state",
              meeting_id: started.id,
              status: "recording",
              paused: false,
            });
            return started;
          }
          case "meetings_cancel_start": {
            // Spiegel von `Recorder::cancel_start`: der Abbruch gilt genau dem Start mit
            // dieser Vorgangs-Id; der Start raeumt auf und meldet `start_cancelled`.
            w.__calls.push({ cmd, args });
            if (w.__cancelErrorOnce) {
              const code = w.__cancelErrorOnce;
              w.__cancelErrorOnce = null;
              throw code;
            }
            const held = w.__heldStart;
            if (held && held.opId === args.opId) {
              w.__heldStart = null;
              // Rollback des Backends: was die Live-Pipeline schon gespeichert und gemeldet
              // hatte, ist verworfen; der Eintrag ist wieder leer und bereit.
              w.__ytSegments.length = 0;
              if (w.__rollbackEmits) {
                w.__emit("meeting-event", {
                  kind: "reset",
                  meeting_id: held.meeting.id,
                });
                w.__emit("meeting-event", {
                  kind: "state",
                  meeting_id: held.meeting.id,
                  status: "ready",
                  paused: false,
                });
              }
              if (w.__holdFinishesOnCancel) held.resolve(held.meeting);
              else held.reject("start_cancelled");
            }
            return null;
          }
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
      empty: options.empty ?? false,
      privateOn: options.privateOn ?? false,
      startError: options.startError ?? null,
      startHold: options.startHold ?? false,
      cancelErrorOnce: options.cancelErrorOnce ?? null,
      rollbackEmits: options.rollbackEmits ?? false,
      holdFinishesOnCancel: options.holdFinishesOnCancel ?? false,
      ownAudio: options.ownAudio ?? false,
      id: YT_ID,
      title: YT_TITLE,
      channel: YT_CHANNEL,
      meetingId: YT_MEETING_ID,
    },
  );
  return { youtubeRequests: requests };
};

/**
 * Beim Mitschreiben kommt ein Satz des Systemtons (Kanal 1) in `y1` an: wird
 * gemerkt (ein Neuladen der Saetze liefert ihn) und als Ereignis gemeldet.
 */
export const captureSegment = (
  page: Page,
  index: number,
  text: string,
  startMs = index * 6_000,
) =>
  page.evaluate(
    ({ meetingId, index, text, startMs }) => {
      const w = window as any;
      const segment = {
        segment_index: index,
        text,
        start_ms: startMs,
        end_ms: startMs + 5_000,
        channel: 1,
        speaker_index: null,
      };
      w.__ytSegments.push(segment);
      w.__emit("meeting-event", {
        kind: "segments",
        meeting_id: meetingId,
        appended: [segment],
      });
    },
    { meetingId: YT_MEETING_ID, index, text, startMs },
  );

/**
 * lva#538 (Runde 4): ab jetzt (`on`) werden Antworten von `meetings_get_segments` fuer `y1`
 * zurueckgehalten, bis `releaseHeldLoads` sie in der Reihenfolge der Anfragen abliefert.
 */
export const holdLoads = (page: Page, on: boolean) =>
  page.evaluate((on) => {
    (window as any).__holdLoads = on;
  }, on);

/** So viele Ladeantworten warten gerade. */
export const heldLoadCount = (page: Page): Promise<number> =>
  page.evaluate(() => (window as any).__heldLoads.length as number);

/** Liefert alle zurueckgehaltenen Ladeantworten ab; Anzahl der abgelieferten. */
export const releaseHeldLoads = (page: Page): Promise<number> =>
  page.evaluate(() => {
    const w = window as any;
    const held = (w.__heldLoads as Array<() => void>).splice(0);
    for (const release of held) release();
    return held.length;
  });

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
