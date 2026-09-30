import type { Page } from "@playwright/test";
import { installRecMock } from "./recLayoutMock";

/**
 * Attrappe fuer Live und Import (Goal aufnahmen-ui, M5): legt ueber die
 * Attrappe der Aufnahmen-Seite die Befehle, die eine laufende Aufnahme und
 * einen Import ausmachen.
 *
 * - `meetings_start` legt die Besprechung `m-neu` (Status "recording") in die
 *   Liste und meldet den Zustand, wie es das Backend tut. `__startError`
 *   laesst den Start scheitern.
 * - `meetings_stop` setzt sie auf "processing" und meldet es.
 * - Segmente: `emitLive` schreibt erst in die "Datenbank" (`__db`), dann in den
 *   Ereignisstrom (wie das Backend: erst speichern, dann melden).
 *   `meetings_get_segments` liefert den Stand der Datenbank zum Zeitpunkt des
 *   AUFRUFS; `holdSegments()` haelt die Antwort zurueck, bis `releaseSegments()`
 *   kommt (Ereignisse waehrend des Ladens).
 * - `meetings_import_file` legt eine Besprechung mit `source_path` an, meldet
 *   "processing" und kehrt erst zurueck, wenn `releaseImport()` kommt (bei
 *   `holdImport()`), sonst sofort: wie der echte Befehl, der bis zum Ende der
 *   Verarbeitung laeuft.
 * - Ordner: Zaehler, Loeschen und das Filtern der Suche nach Projekt.
 */

export const seg = (index: number, text: string, channel = index % 2) => ({
  segment_index: index,
  text,
  start_ms: index * 5_000 + 1_000,
  end_ms: index * 5_000 + 4_000,
  channel,
  speaker_index: null,
});

export const installLiveMock = async (
  page: Page,
  options: { recording?: boolean } = {},
) => {
  await installRecMock(page, options);
  await page.addInitScript(() => {
    const w = window as any;
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__db = {};
    w.__importSeq = 0;
    w.__startError = null;
    w.__importHold = false;
    w.__segmentsGate = null;
    w.__holdSegments = () => {
      w.__segmentsGate = new Promise<void>((resolve) => {
        w.__releaseSegments = () => {
          w.__segmentsGate = null;
          resolve();
        };
      });
    };
    w.__holdImport = () => (w.__importHold = true);
    const emit = (payload: unknown) => w.__emit("meeting-event", payload);
    const live = (id: string) => w.__folders.some((f: any) => f.id === id);
    const assigned = (id: string) => (w.__folderMap[id] ?? []).filter(live);

    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, any> = {},
    ) => {
      switch (cmd) {
        case "meetings_start": {
          w.__calls.push({ cmd, args });
          if (w.__startError) throw w.__startError;
          const created = {
            ...w.__meetings[0],
            id: "m-neu",
            title: args.title,
            status: "recording",
            ended_at: null,
            duration_ms: null,
            started_at: Math.floor(Date.now() / 1000),
            mic_audio_path: null,
            system_audio_path: null,
          };
          w.__meetings = [created, ...w.__meetings];
          w.__recording = true;
          w.__position = { meeting_id: "m-neu", position_ms: 2_000 };
          setTimeout(
            () =>
              emit({
                kind: "state",
                meeting_id: "m-neu",
                status: "recording",
                paused: false,
              }),
            30,
          );
          return created;
        }
        case "meetings_stop": {
          w.__calls.push({ cmd, args });
          w.__recording = false;
          w.__position = null;
          w.__meetings = w.__meetings.map((m: any) =>
            m.id === "m-neu" ? { ...m, status: "processing" } : m,
          );
          setTimeout(
            () =>
              emit({
                kind: "state",
                meeting_id: "m-neu",
                status: "processing",
                paused: false,
              }),
            30,
          );
          return "m-neu";
        }
        case "meetings_get_segments": {
          if (!(args.meetingId in w.__db) && args.meetingId !== "m-neu") break;
          w.__calls.push({ cmd, args });
          const snapshot = [...(w.__db[args.meetingId] ?? [])];
          const gate = w.__segmentsGate;
          if (gate) await gate;
          return snapshot;
        }
        case "meetings_import_file": {
          w.__calls.push({ cmd, args });
          const id = `m-imp${++w.__importSeq}`;
          const stem = String(args.path)
            .split(/[\\/]/)
            .pop()!
            .replace(/\.[^.]+$/, "");
          const created = {
            ...w.__meetings[0],
            id,
            title: stem,
            status: "processing",
            source: "import",
            source_path: args.path,
            ended_at: null,
            duration_ms: null,
            started_at: Math.floor(Date.now() / 1000),
            mic_audio_path: null,
            system_audio_path: null,
          };
          w.__meetings = [created, ...w.__meetings];
          setTimeout(
            () =>
              emit({
                kind: "state",
                meeting_id: id,
                status: "processing",
                paused: false,
              }),
            10,
          );
          if (w.__importHold) {
            await new Promise<void>((resolve) => {
              w.__releaseImport = () => {
                w.__importHold = false;
                resolve();
              };
            });
          }
          w.__meetings = w.__meetings.map((m: any) =>
            m.id === id ? { ...m, status: "ready" } : m,
          );
          emit({
            kind: "state",
            meeting_id: id,
            status: "ready",
            paused: false,
          });
          return id;
        }
        case "meeting_folders_list":
          return w.__folders.map((f: any) => ({
            ...f,
            meeting_count: w.__meetings.filter((m: any) =>
              assigned(m.id).includes(f.id),
            ).length,
          }));
        case "meeting_folders_counts":
          return {
            all: w.__meetings.length,
            unfiled: w.__meetings.filter(
              (m: any) => assigned(m.id).length === 0,
            ).length,
          };
        case "meeting_folders_delete":
          w.__calls.push({ cmd, args });
          w.__folders = w.__folders.filter((f: any) => f.id !== args.id);
          return null;
        case "meetings_search": {
          const f = args.filter ?? {};
          const items = w.__meetings
            .filter((m: any) => {
              const ids = assigned(m.id);
              if (f.unfiled && ids.length > 0) return false;
              if (f.folder_id && !ids.includes(f.folder_id)) return false;
              return true;
            })
            .map((m: any) => ({ meeting: m, snippet: null, hit_source: null }));
          return { items, total: items.length, truncated: false };
        }
        case "meetings_get_folders":
          return assigned(args.meetingId);
        case "meetings_set_folders":
          w.__calls.push({ cmd, args });
          w.__folderMap[args.meetingId] = args.folderIds;
          return null;
      }
      return inner(cmd, args);
    };
  });
};

/** Segmente "speichern" und melden, wie das Backend es tut. */
export const emitLive = (
  page: Page,
  meetingId: string,
  segments: ReturnType<typeof seg>[],
) =>
  page.evaluate(
    ({ id, list }) => {
      const w = window as any;
      w.__db[id] = [...(w.__db[id] ?? []), ...list];
      w.__emit("meeting-event", {
        kind: "segments",
        meeting_id: id,
        appended: list,
      });
    },
    { id: meetingId, list: segments },
  );

/** Nur melden (ohne zu speichern): ein Ereignis, dessen Satz schon im Stand der Datenbank steckt. */
export const emitOnly = (
  page: Page,
  meetingId: string,
  segments: ReturnType<typeof seg>[],
) =>
  page.evaluate(
    ({ id, list }) =>
      (window as any).__emit("meeting-event", {
        kind: "segments",
        meeting_id: id,
        appended: list,
      }),
    { id: meetingId, list: segments },
  );

export const holdSegments = (page: Page) =>
  page.evaluate(() => (window as any).__holdSegments());
export const releaseSegments = (page: Page) =>
  page.evaluate(() => (window as any).__releaseSegments());
export const holdImport = (page: Page) =>
  page.evaluate(() => (window as any).__holdImport());
export const releaseImport = (page: Page) =>
  page.evaluate(() => (window as any).__releaseImport());

/** Aufnahmezeit fuer den naechsten Notizblock (Stempel). */
export const setPosition = (page: Page, ms: number) =>
  page.evaluate(
    (position) =>
      ((window as any).__position = {
        meeting_id: "m-neu",
        position_ms: position,
      }),
    ms,
  );

/** Startdialog bestaetigen (Einwilligung). */
export const confirmStartDialog = async (page: Page) => {
  await page
    .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
    .click();
};

/** Aufnahme ueber Knopf und Startdialog starten. */
export const startRecording = async (page: Page) => {
  await page
    .getByRole("button", { name: /Aufnahme starten/ })
    .first()
    .click();
  await page.getByTestId("start-dialog").waitFor();
  await confirmStartDialog(page);
};
