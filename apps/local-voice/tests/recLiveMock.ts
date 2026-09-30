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
 * - `meetings_import_file` (U7, Warteschlange) legt eine Besprechung mit
 *   `source_path` an (Status "queued") und kehrt SOFORT mit ihrer ID zurueck,
 *   wie der echte Befehl. Die Warteschlange der Attrappe beginnt die naechste
 *   Datei, sobald ein Platz frei ist (`limit`, Standard 1) und meldet
 *   `state` (processing/ready) und den Stand als `import-queue-event`. Ohne
 *   `holdImport()` endet jeder Lauf nach 30 ms; mit `holdImport()` laeuft er,
 *   bis `finishImport(id)` oder `releaseImport()` kommt.
 * - Warteschlangen-Befehle: `meetings_queue_list/remove/to_front`;
 *   `setQueueLimit`, `setQueueMemory` ("wartet auf Arbeitsspeicher") und
 *   `setQueueRecording` (Aufnahme-Vorrang) stellen die Lage nach.
 * - `meetings_update_metadata` schreibt Titel, Beschreibung, Datum,
 *   Teilnehmende und Projekte in die Attrappe (mit `persist`: ueber ein Neuladen
 *   der Seite hinweg, per sessionStorage) und kennt Fehler (`__metaError`).
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

export const PEOPLE = [
  {
    id: "h-anna",
    name: "Anna Berg",
    email: "anna@firma.de",
    company: "Firma GmbH",
    is_self: false,
    meeting_count: 4,
  },
  {
    id: "h-ben",
    name: "Ben Koch",
    email: "ben@kunde.de",
    company: "Kunde AG",
    is_self: false,
    meeting_count: 2,
  },
  {
    id: "h-cem",
    name: "Cem Aydin",
    email: null,
    company: null,
    is_self: false,
    meeting_count: 1,
  },
];

export const installLiveMock = async (
  page: Page,
  options: { recording?: boolean; persist?: boolean } = {},
) => {
  await installRecMock(page, options);
  await page.addInitScript((persistState) => {
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

    // U7: Warteschlange der Attrappe (ein Platz, `limit` Plaetze).
    w.__queue = {
      waiting: [] as string[],
      running: [] as string[],
      held: [] as string[],
      limit: 1,
      memory: false,
      recording: false,
    };
    w.__queueSnapshot = () => {
      const q = w.__queue;
      let blocked: string | null = null;
      if (q.waiting.length > 0) {
        blocked = q.recording
          ? "recording"
          : q.running.length > 0 && q.running.length < q.limit && q.memory
            ? "memory"
            : q.running.length >= q.limit
              ? "slot"
              : null;
      }
      return {
        waiting: [...q.waiting],
        running: [...q.running],
        held: [...q.held],
        limit: q.limit,
        blocked,
      };
    };
    w.__queuePublish = () =>
      w.__emit("import-queue-event", { snapshot: w.__queueSnapshot() });
    w.__queueSetStatus = (id: string, status: string) => {
      w.__meetings = w.__meetings.map((m: any) =>
        m.id === id ? { ...m, status } : m,
      );
      emit({ kind: "state", meeting_id: id, status, paused: false });
    };
    w.__finishImport = (id: string) => {
      const q = w.__queue;
      if (!q.running.includes(id)) return;
      q.running = q.running.filter((x: string) => x !== id);
      q.held = q.held.filter((x: string) => x !== id);
      w.__queueSetStatus(id, "ready");
      w.__queueStart();
    };
    w.__queueStart = () => {
      const q = w.__queue;
      while (
        q.waiting.length > 0 &&
        !q.recording &&
        q.running.length < q.limit &&
        (q.running.length === 0 || !q.memory)
      ) {
        const id = q.waiting.shift() as string;
        q.running.push(id);
        w.__queueSetStatus(id, "processing");
        if (!w.__importHold) setTimeout(() => w.__finishImport(id), 30);
      }
      w.__queuePublish();
    };
    w.__releaseImport = () => {
      w.__importHold = false;
      [...w.__queue.running].forEach((id: string) => w.__finishImport(id));
    };
    w.__setQueueRecording = (on: boolean) => {
      const q = w.__queue;
      q.recording = on;
      q.held = on ? [...q.running] : [];
      if (on) w.__queuePublish();
      else w.__queueStart();
    };

    // U7: Metadaten ueber ein Neuladen hinweg (sessionStorage), wenn verlangt.
    w.__people = [];
    w.__persist = () => {
      if (!persistState) return;
      try {
        sessionStorage.setItem(
          "__u7_state",
          JSON.stringify({
            meetings: w.__meetings,
            participants: w.__participants,
            folderMap: w.__folderMap,
          }),
        );
      } catch {
        /* ohne Speicher laeuft die Attrappe ohne Neuladen-Test */
      }
    };
    if (persistState) {
      try {
        const saved = sessionStorage.getItem("__u7_state");
        if (saved) {
          const state = JSON.parse(saved);
          w.__meetings = state.meetings;
          w.__participants = state.participants;
          w.__folderMap = state.folderMap;
        }
      } catch {
        /* kein gespeicherter Stand */
      }
    }
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
          // Neu importierte Besprechungen (m-imp*) haben keine Segmente, bis der Test welche meldet.
          if (
            !(args.meetingId in w.__db) &&
            args.meetingId !== "m-neu" &&
            !String(args.meetingId).startsWith("m-imp")
          )
            break;
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
            status: "queued",
            source: "import",
            source_path: args.path,
            description: null,
            ended_at: null,
            duration_ms: null,
            started_at: Math.floor(Date.now() / 1000),
            mic_audio_path: null,
            system_audio_path: null,
          };
          w.__meetings = [created, ...w.__meetings];
          w.__queue.waiting.push(id);
          // Wie das Backend: die Besprechung gibt es sofort, der Verteiler beginnt kurz darauf.
          setTimeout(() => w.__queueStart(), 10);
          return id;
        }
        case "meetings_continue": {
          // U7: eine aus der Warteschlange genommene Datei ohne Audio wird hinten wieder eingereiht.
          w.__calls.push({ cmd, args });
          const m = w.__meetings.find((x: any) => x.id === args.meetingId);
          if (!m || m.status !== "cancelled") throw "not_cancelled";
          w.__queue.waiting.push(m.id);
          w.__queueSetStatus(m.id, "queued");
          setTimeout(() => w.__queueStart(), 10);
          return null;
        }
        case "meetings_job_stop": {
          // Stopp eines laufenden Imports: Endzustand "cancelled", die naechste Datei beginnt.
          w.__calls.push({ cmd, args });
          const q = w.__queue;
          if (!q.running.includes(args.meetingId)) throw "no_job";
          q.running = q.running.filter((x: string) => x !== args.meetingId);
          q.held = q.held.filter((x: string) => x !== args.meetingId);
          w.__queueSetStatus(args.meetingId, "cancelled");
          emit({
            kind: "job_ended",
            meeting_id: args.meetingId,
            phase: "transcription",
            stopped: true,
          });
          w.__queueStart();
          return null;
        }
        case "meetings_queue_list":
          return w.__queueSnapshot();
        case "meetings_queue_remove": {
          w.__calls.push({ cmd, args });
          const q = w.__queue;
          const id = args.meetingId as string;
          if (q.waiting.includes(id)) {
            q.waiting = q.waiting.filter((x: string) => x !== id);
            w.__queueSetStatus(id, "cancelled");
            w.__queuePublish();
            return null;
          }
          if (q.running.includes(id)) {
            q.running = q.running.filter((x: string) => x !== id);
            w.__queueSetStatus(id, "cancelled");
            w.__queueStart();
            return null;
          }
          throw "not_in_queue";
        }
        case "meetings_queue_to_front": {
          w.__calls.push({ cmd, args });
          const q = w.__queue;
          const id = args.meetingId as string;
          if (!q.waiting.includes(id)) throw "not_queued";
          q.waiting = [id, ...q.waiting.filter((x: string) => x !== id)];
          w.__queuePublish();
          return null;
        }
        case "people_list":
          return w.__people ?? [];
        case "meetings_update_metadata": {
          w.__calls.push({ cmd, args });
          if (w.__metaError) throw w.__metaError;
          const edit = args.edit;
          const current = w.__meetings.find(
            (m: any) => m.id === args.meetingId,
          );
          if (!current) throw "meeting_not_found";
          if (edit.title !== null && edit.title.trim() === "")
            throw "title_empty";
          const next = { ...current };
          if (edit.title !== null) next.title = edit.title.trim();
          if (edit.description !== null) {
            next.description = edit.description.trim() || null;
          }
          if (edit.started_at !== null) next.started_at = edit.started_at;
          w.__meetings = w.__meetings.map((m: any) =>
            m.id === next.id ? next : m,
          );
          if (edit.participant_ids !== null) {
            w.__participants[next.id] = edit.participant_ids.map(
              (pid: string) => {
                const p = (w.__people ?? []).find((x: any) => x.id === pid);
                return {
                  human_id: pid,
                  name: p?.name ?? pid,
                  email: p?.email ?? null,
                  company: p?.company ?? null,
                  role: "attendee",
                  source: "manual",
                  is_self: false,
                  meeting_count: 1,
                };
              },
            );
          }
          if (edit.folder_ids !== null) {
            w.__folderMap[next.id] = edit.folder_ids;
          }
          w.__persist();
          return next;
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
  }, options.persist ?? false);
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

/** Einen laufenden Import beenden (Status "ready"); die naechste Datei beginnt. */
export const finishImport = (page: Page, id: string) =>
  page.evaluate((i) => (window as any).__finishImport(i), id);

/** Zahl gleichzeitiger Laeufe der Attrappe (Einstellung). */
export const setQueueLimit = (page: Page, limit: number) =>
  page.evaluate((n) => {
    const w = window as any;
    w.__queue.limit = n;
    w.__queueStart();
  }, limit);

/** Kein Speicher fuer einen weiteren gleichzeitigen Lauf ("wartet auf Arbeitsspeicher"). */
export const setQueueMemory = (page: Page, blocked: boolean) =>
  page.evaluate((on) => {
    const w = window as any;
    w.__queue.memory = on;
    w.__queueStart();
  }, blocked);

/** Eine Aufnahme laeuft: die Warteschlange haelt an (und setzt danach fort). */
export const setQueueRecording = (page: Page, on: boolean) =>
  page.evaluate((v) => (window as any).__setQueueRecording(v), on);

/** Personen der Attrappe (Teilnehmenden-Auswahl). */
export const setPeople = (page: Page, people: typeof PEOPLE) =>
  page.evaluate((list) => ((window as any).__people = list), people);

/** Beim Neuladen bleibt der Stand der Attrappe erhalten (`persist`): Hilfe fuer "nach Neuladen". */
export const reloadKeepingMock = async (page: Page) => {
  await page.reload();
};
