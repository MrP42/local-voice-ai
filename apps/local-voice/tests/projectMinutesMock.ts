import type { Page } from "@playwright/test";

/**
 * Attrappe des Projekt-Protokolls (G3, #70, U9) fuer `project-minutes.spec.ts`.
 * Sie legt sich ueber `installLiveMock` und spielt nur nach, was das Backend fuer
 * die neuen Befehle tut (die Regeln selbst -- Auswahl, Reihenfolge, Belege,
 * Ablehnung -- pruefen die Rust-Tests in `minutes/project/`):
 *
 * - Das Projekt `f2` ("Geschaeftlich") enthaelt sechs Aufnahmen: m1, m2, m4 mit
 *   Transkript und Ton, m5 ohne Transkript, m3 in Verarbeitung und einen leeren
 *   Eintrag m7. `meetings_get_segments` liefert fuer m5 und m7 nichts.
 * - `project_minutes_candidates` meldet je Aufnahme "waehlbar" oder den Grund
 *   (`no_transcript`, `meeting_not_finished`, `empty_entry`).
 * - `project_minutes_generate` lehnt eine nicht waehlbare Aufnahme ab (wie das
 *   Backend: `<code>: <id>`), sonst laeuft er als Auftrag unter dem Schluessel
 *   `project-minutes:<projekt>` (Fortschritt ueber `meeting-event`, Ende mit
 *   `project-minutes-event`). Mit `holdProjectMinutes` bleibt er im Lauf, bis
 *   `releaseProjectMinutes` kommt; `meetings_job_stop` auf dem Schluessel beendet
 *   ihn mit `minutes_cancelled`.
 * - Das Ergebnis enthaelt Abschnitte mit Belegen (Aufnahme, Segment, Zeit), eine
 *   Aussage ohne Beleg und je Aufnahme die Quelle; `project_minutes_list/get/delete`
 *   verwalten die Dokumente (`w.__pm.docs`).
 */
export const PM_FOLDER = "f2";
export const PM_FOLDER_NAME = "Geschäftlich";
export const PM_KEY = `project-minutes:${PM_FOLDER}`;

export const installProjectMinutesMock = async (page: Page) => {
  await page.addInitScript(
    ({ folderId, folderName }) => {
      const w = window as any;
      const inner = w.__TAURI_INTERNALS__.invoke;
      const T0 = 1_790_000_000;

      // Daten: ein leerer Eintrag, Ton fuer die Aufnahmen mit Transkript.
      w.__meetings = [
        ...w.__meetings,
        {
          ...w.__meetings[0],
          id: "m7",
          title: "Neue Besprechung",
          status: "ready",
          source: "empty",
          started_at: null,
          ended_at: null,
          language: null,
          mic_audio_path: null,
          system_audio_path: null,
          duration_ms: null,
          source_path: null,
          created_at: T0 + 6 * 86400,
        },
      ];
      for (const id of ["m1", "m4"]) {
        const m = w.__meetings.find((x: any) => x.id === id);
        m.mic_audio_path = `C:/audio/${id}-mic.wav`;
      }
      w.__folderMap = {
        ...w.__folderMap,
        m1: [folderId],
        m2: [folderId, "f3"],
        m3: [folderId],
        m4: [folderId],
        m5: [folderId],
        m7: [folderId],
      };

      // Wiedergabe nur mitschreiben: die Position beim play() ist das Sprungziel.
      w.__played = [];
      const times = new WeakMap<object, number>();
      Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
        configurable: true,
        get() {
          return times.get(this) ?? 0;
        },
        set(v: number) {
          times.set(this, v);
        },
      });
      HTMLMediaElement.prototype.play = function () {
        w.__played.push(times.get(this) ?? 0);
        return Promise.resolve();
      };
      HTMLMediaElement.prototype.pause = function () {};

      // Beim Neuladen bleiben die Dokumente stehen (das Backend speichert sie).
      const stored = (() => {
        try {
          return JSON.parse(sessionStorage.getItem("__pmDocs") ?? "[]");
        } catch {
          return [];
        }
      })();
      const persistDocs = () => {
        try {
          sessionStorage.setItem("__pmDocs", JSON.stringify(w.__pm.docs));
        } catch {
          /* nicht merken ist verkraftbar */
        }
      };
      w.__pm = {
        docs: stored as any[],
        seq: stored.length,
        hold: false,
        release: null as null | (() => void),
        cancel: false,
        running: {} as Record<string, boolean>,
        failWith: null as null | string,
      };
      w.__holdPm = () => (w.__pm.hold = true);
      w.__releasePm = () => {
        w.__pm.hold = false;
        w.__pm.release?.();
      };

      const NO_TRANSCRIPT = new Set(["m5", "m7"]);
      const meetingOf = (id: string) =>
        w.__meetings.find((m: any) => m.id === id);
      const members = (folder: string) =>
        w.__meetings
          .filter((m: any) => (w.__folderMap[m.id] ?? []).includes(folder))
          .sort(
            (a: any, b: any) =>
              (a.started_at ?? a.created_at) - (b.started_at ?? b.created_at),
          );
      const candidateOf = (m: any) => {
        const segments = NO_TRANSCRIPT.has(m.id) ? 0 : w.__segments.length;
        let reason: string | null = null;
        if (!["ready", "failed", "cancelled"].includes(m.status))
          reason = "meeting_not_finished";
        else if (segments === 0)
          reason = m.source === "empty" ? "empty_entry" : "no_transcript";
        return {
          meeting_id: m.id,
          eligible: reason === null,
          reason,
          segments,
        };
      };
      const emit = (event: string, payload: unknown) =>
        w.__emit(event, payload);
      const progress = (key: string, done: number, total: number) => {
        const p = {
          meeting_id: key,
          phase: "minutes",
          done,
          total,
          elapsed_ms: 4_000,
          eta_ms: null,
          state: "running",
          pausable: false,
        };
        w.__progressList = [
          ...w.__progressList.filter((x: any) => x.meeting_id !== key),
          p,
        ];
        emit("meeting-event", { kind: "progress", ...p });
      };
      const endJob = (key: string, stopped: boolean) => {
        w.__progressList = w.__progressList.filter(
          (x: any) => x.meeting_id !== key,
        );
        emit("meeting-event", {
          kind: "job_ended",
          meeting_id: key,
          phase: "minutes",
          stopped,
        });
      };
      const seg = (index: number) => index * 29_000 + 4_000;
      const markdown = (title: string, recs: any[]) =>
        [
          `# ${title}`,
          "",
          `**Zeitraum:** 2026-09-28 bis 2026-10-02 · **Aufnahmen:** ${recs.length}`,
          "",
          "## Zusammenfassung",
          "",
          "Die Lastspitzen liegen morgens, ein Speicher glättet sie. [A1 00:33]",
        ].join("\n");

      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        switch (cmd) {
          case "meetings_get_segments":
            if (NO_TRANSCRIPT.has(args.meetingId)) {
              w.__calls.push({ cmd, args });
              return [];
            }
            break;
          case "plugin:dialog|save": {
            w.__calls.push({ cmd, args });
            return "C:/Temp/Projekt-Protokoll.docx";
          }
          case "project_minutes_candidates": {
            w.__calls.push({ cmd, args });
            if (!w.__folders.some((f: any) => f.id === args.folderId))
              throw "folder_not_found";
            return members(args.folderId).map(candidateOf);
          }
          case "project_minutes_list": {
            w.__calls.push({ cmd, args });
            return w.__pm.docs
              .filter((d: any) => d.folder_id === args.folderId)
              .sort((a: any, b: any) => b.created_at - a.created_at)
              .map((d: any) => ({
                id: d.id,
                folder_id: d.folder_id,
                kind: d.kind,
                title: d.title,
                created_at: d.created_at,
                recordings: d.recordings.length,
                template_title: d.meta.template_title,
                incomplete: d.meta.incomplete,
              }));
          }
          case "project_minutes_get": {
            w.__calls.push({ cmd, args });
            return w.__pm.docs.find((d: any) => d.id === args.id) ?? null;
          }
          case "project_minutes_delete": {
            w.__calls.push({ cmd, args });
            const had = w.__pm.docs.some((d: any) => d.id === args.id);
            w.__pm.docs = w.__pm.docs.filter((d: any) => d.id !== args.id);
            persistDocs();
            return had;
          }
          case "project_minutes_state": {
            const running = !!w.__pm.running[args.folderId];
            return {
              running,
              progress: null,
              cancelling: running && w.__pm.cancel,
              started_at: running ? Math.floor(Date.now() / 1000) : null,
            };
          }
          case "project_minutes_cancel": {
            w.__calls.push({ cmd, args });
            w.__pm.cancel = true;
            w.__releasePm();
            return !!w.__pm.running[args.folderId];
          }
          case "meetings_job_stop": {
            if (String(args.meetingId).startsWith("project-minutes:")) {
              w.__calls.push({ cmd, args });
              w.__pm.cancel = true;
              w.__pm.release?.();
              return null;
            }
            break;
          }
          case "project_minutes_generate": {
            w.__calls.push({ cmd, args });
            const key = `project-minutes:${args.folderId}`;
            if (w.__pm.running[args.folderId]) throw "minutes_busy";
            if (!["minutes", "summary"].includes(args.kind))
              throw "kind_invalid";
            const ids: string[] = [...new Set<string>(args.meetingIds)];
            if (ids.length === 0) throw "no_selection";
            for (const id of ids) {
              const m = meetingOf(id);
              if (!m) throw `meeting_not_found: ${id}`;
              if (!(w.__folderMap[id] ?? []).includes(args.folderId))
                throw `not_in_project: ${id}`;
              const c = candidateOf(m);
              if (!c.eligible)
                throw `${c.reason === "meeting_not_finished" ? "meeting_not_finished" : "no_transcript"}: ${id}`;
            }
            w.__pm.running[args.folderId] = true;
            w.__pm.cancel = false;
            progress(key, 0, 0);
            await new Promise((r) => setTimeout(r, 40));
            progress(key, 1, 4);
            if (w.__pm.hold)
              await new Promise<void>((resolve) => {
                w.__pm.release = resolve;
              });
            const fail = async (code: string) => {
              w.__pm.running[args.folderId] = false;
              endJob(key, code === "minutes_cancelled");
              emit("project-minutes-event", {
                kind: "failed",
                folder_id: args.folderId,
                code,
                detail: "",
              });
              throw code;
            };
            if (w.__pm.cancel) await fail("minutes_cancelled");
            if (w.__pm.failWith) await fail(w.__pm.failWith);
            progress(key, 4, 4);
            // Chronologisch: Aufnahme 1 ist die aelteste.
            const ordered = ids
              .map(meetingOf)
              .sort(
                (a: any, b: any) =>
                  (a.started_at ?? a.created_at) -
                  (b.started_at ?? b.created_at),
              );
            const recordings = ordered.map((m: any, i: number) => ({
              index: i + 1,
              meeting_id: m.id,
              title: m.title,
              started_at: m.started_at ?? m.created_at,
              duration_ms: m.duration_ms,
              segments: w.__segments.length,
            }));
            const src = (r: number, segment: number) => ({
              recording: r,
              meeting_id: recordings[r - 1].meeting_id,
              segment_index: segment,
              start_ms: seg(segment),
            });
            const last = recordings.length;
            const summary = args.kind === "summary";
            const name = summary
              ? `Projekt-Zusammenfassung: ${folderName}`
              : `Projekt-Protokoll: ${folderName}`;
            const doc = {
              id: `PM${++w.__pm.seq}`,
              folder_id: args.folderId,
              kind: args.kind,
              title: name,
              body: markdown(name, recordings),
              sections: [
                {
                  id: "zusammenfassung",
                  title: "Zusammenfassung",
                  kind: "text",
                  entries: [
                    {
                      text: "Die Lastspitzen liegen morgens, ein Speicher glättet sie.",
                      assignee: null,
                      due: null,
                      sources: [src(1, 1), src(last, 5)],
                      unsupported: false,
                    },
                  ],
                },
                {
                  id: "entscheidungen",
                  title: "Entscheidungen",
                  kind: "text",
                  entries: [
                    {
                      text: "Der Speicher wird geprüft.",
                      assignee: null,
                      due: null,
                      sources: [src(last, 3)],
                      unsupported: false,
                    },
                    {
                      text: "Das Budget ist offen.",
                      assignee: null,
                      due: null,
                      sources: [],
                      unsupported: true,
                    },
                  ],
                },
                {
                  id: "aufgaben",
                  title: "Aufgaben",
                  kind: "tasks",
                  entries: [
                    {
                      text: "Kurzfassung schicken",
                      assignee: "Patrick",
                      due: "Freitag",
                      sources: [src(1, 7)],
                      unsupported: false,
                    },
                  ],
                },
              ],
              recordings,
              meta: {
                model: "test-model",
                provider: "custom",
                template_id: args.templateId ?? "builtin:allgemein",
                template_title:
                  args.templateId === "auto"
                    ? "Kundengespräch / Vertrieb"
                    : "Allgemein",
                auto:
                  args.templateId === "auto"
                    ? {
                        template_id: "builtin:vertrieb",
                        title: "Kundengespräch / Vertrieb",
                        reason: "Kundentermine",
                        outcome: "model",
                      }
                    : null,
                single_pass: true,
                chunks_total: 1,
                chunks_split: 0,
                incomplete: false,
                gaps: [],
                dropped_sources: 1,
                unsupported_entries: 1,
              },
              created_at: Math.floor(Date.now() / 1000),
            };
            w.__pm.docs.push(doc);
            persistDocs();
            w.__pm.running[args.folderId] = false;
            endJob(key, false);
            emit("project-minutes-event", {
              kind: "done",
              folder_id: args.folderId,
              minutes_id: doc.id,
            });
            return doc;
          }
        }
        return inner(cmd, args);
      };
    },
    { folderId: PM_FOLDER, folderName: PM_FOLDER_NAME },
  );
};

/** Der Lauf bleibt stehen, bis `releaseProjectMinutes` kommt. */
export const holdProjectMinutes = (page: Page) =>
  page.evaluate(() => (window as any).__holdPm());

export const releaseProjectMinutes = (page: Page) =>
  page.evaluate(() => (window as any).__releasePm());

/** Der naechste Lauf scheitert mit diesem Code (Ereignis und Ergebnis). */
export const failProjectMinutes = (page: Page, code: string | null) =>
  page.evaluate((c) => ((window as any).__pm.failWith = c), code);
