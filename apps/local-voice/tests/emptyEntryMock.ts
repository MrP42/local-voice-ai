import type { Page } from "@playwright/test";

/**
 * Attrappe des leeren Eintrags (G1, #70) fuer `meeting-empty-entry.spec.ts`.
 * Sie legt sich ueber `installLiveMock` und `installYoutubeMock` und spielt nur
 * nach, was das Backend fuer einen leeren Eintrag tut (die Regeln selbst, auch
 * die Ablehnung eines nicht leeren Ziels, pruefen die Rust-Tests):
 *
 * - `meetings_create_empty` legt eine Besprechung mit `source = "empty"`
 *   (Status "ready", ohne Audio) an, auf Wunsch gleich im Projekt, und merkt
 *   sich den vorgeschlagenen Titel.
 * - `meetings_start`, `meetings_import_file`, `youtube_add_source` mit
 *   `targetMeetingId` wandeln DIESEN Eintrag um (Aufnahme / Import / YouTube);
 *   der Titel wird beim Import und beim Link nur ersetzt, solange er noch der
 *   vorgeschlagene ist. Ein anderes Ziel (nicht leer, unbekannt, oder
 *   `__targetError`) wird mit `target_not_empty` bzw. `meeting_not_found`
 *   abgelehnt.
 * - Ohne `targetMeetingId` reicht sie alles an die darunterliegende Attrappe.
 */
export const installEmptyEntryMock = async (page: Page) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__emptySeq = 0;
    w.__defaultTitles = {} as Record<string, string>;
    w.__targetError = false;
    const inner = w.__TAURI_INTERNALS__.invoke;
    const emit = (payload: unknown) => w.__emit("meeting-event", payload);
    const find = (id: string) => w.__meetings.find((m: any) => m.id === id);
    const patch = (id: string, over: Record<string, unknown>) => {
      w.__meetings = w.__meetings.map((m: any) =>
        m.id === id ? { ...m, ...over } : m,
      );
      return find(id);
    };
    const requireEmpty = (id: string) => {
      const m = find(id);
      if (!m) throw "meeting_not_found";
      if (w.__targetError || m.source !== "empty") throw "target_not_empty";
      return m;
    };
    const now = () => Math.floor(Date.now() / 1000);

    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, any> = {},
    ) => {
      switch (cmd) {
        case "meetings_get_segments": {
          // Ein leerer Eintrag hat kein Transkript.
          if (find(args.meetingId)?.source === "empty") {
            w.__calls.push({ cmd, args });
            return [];
          }
          break;
        }
        case "meetings_create_empty": {
          w.__calls.push({ cmd, args });
          if (String(args.title).trim() === "") throw "title_empty";
          if (
            args.folderId &&
            !w.__folders.some((f: any) => f.id === args.folderId)
          )
            throw "folder_not_found";
          const id = `m-empty${++w.__emptySeq}`;
          const created = {
            ...w.__meetings[0],
            id,
            title: args.title,
            status: "ready",
            source: "empty",
            started_at: null,
            ended_at: null,
            language: null,
            mic_audio_path: null,
            system_audio_path: null,
            duration_ms: null,
            consent_confirmed_at: null,
            audio_retention_until: null,
            source_path: null,
            description: null,
            created_at: now(),
          };
          w.__meetings = [created, ...w.__meetings];
          if (args.folderId) w.__folderMap[id] = [args.folderId];
          w.__defaultTitles[id] = args.title;
          return created;
        }
        case "meetings_start":
        case "meetings_start_from_event": {
          if (!args.targetMeetingId) break;
          w.__calls.push({ cmd, args });
          if (w.__startError) throw w.__startError;
          const m = requireEmpty(args.targetMeetingId);
          const title = String(args.title ?? "").trim() || m.title;
          const updated = patch(m.id, {
            title,
            status: "recording",
            source: "live",
            consent_confirmed_at: now(),
            started_at: now(),
          });
          w.__recording = true;
          w.__position = { meeting_id: m.id, position_ms: 2_000 };
          setTimeout(
            () =>
              emit({
                kind: "state",
                meeting_id: m.id,
                status: "recording",
                paused: false,
              }),
            30,
          );
          return updated;
        }
        case "meetings_import_file": {
          if (!args.targetMeetingId) break;
          w.__calls.push({ cmd, args });
          const m = requireEmpty(args.targetMeetingId);
          const stem = String(args.path)
            .split(/[\\/]/)
            .pop()!
            .replace(/\.[^.]+$/, "");
          patch(m.id, {
            status: "queued",
            source: "import",
            source_path: args.path,
            consent_confirmed_at: args.consentConfirmed ? now() : null,
            title: m.title === w.__defaultTitles[m.id] ? stem : m.title,
          });
          w.__queue.waiting.push(m.id);
          setTimeout(() => w.__queueStart(), 10);
          return m.id;
        }
        case "youtube_add_source": {
          if (!args.targetMeetingId) break;
          w.__calls.push({ cmd, args });
          const link = await inner("youtube_normalize_link", { url: args.url });
          const m = requireEmpty(args.targetMeetingId);
          const title =
            m.title === w.__defaultTitles[m.id]
              ? "Lastgang verstehen: Spitzen glätten"
              : m.title;
          const updated = patch(m.id, {
            status: "ready",
            source: "youtube",
            title,
          });
          w.__ytSources[m.id] = {
            video_id: link.video_id,
            url: link.url,
            title: "Lastgang verstehen: Spitzen glätten",
            channel: "Wolff Applied AI",
            channel_url: null,
            thumbnail_url: null,
            start_s: link.start_s,
          };
          return updated;
        }
      }
      return inner(cmd, args);
    };
  });
};

/** Das Backend lehnt den Eintrag als Ziel ab (er waere nicht mehr leer). */
export const refuseTarget = (page: Page) =>
  page.evaluate(() => ((window as any).__targetError = true));
