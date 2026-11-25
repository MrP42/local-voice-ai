import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type NoteBlock } from "@/bindings";
import {
  AUTOSAVE_DEBOUNCE_MS,
  blocksForSave,
  mergeConflict,
  notesTooLarge,
} from "@/lib/meetingNotes";

export type NotesSaveStatus =
  "idle" | "dirty" | "saving" | "saved" | "error" | "tooLarge";

type BlocksUpdater = NoteBlock[] | ((prev: NoteBlock[]) => NoteBlock[]);

export interface NotesAutosave {
  blocks: NoteBlock[];
  setBlocks: (next: BlocksUpdater) => void;
  status: NotesSaveStatus;
  /** Anzahl der Konfliktaufloesungen; die UI zeigt einmalig einen Hinweis. */
  conflicts: number;
  loaded: boolean;
  loadError: string | null;
  /** `true`, wenn danach nichts mehr ungesichert ist. */
  flush: () => Promise<boolean>;
}

/** Sichert einen Notizblock sofort; `true`, wenn danach nichts mehr ungesichert ist. */
export type NotesFlusher = () => Promise<boolean>;

const flushers = new Set<NotesFlusher>();

/** Meldet einen Notizblock fuer `flushAllNotes` an; die Rueckgabe meldet ihn ab. */
export const registerNotesFlusher = (flush: NotesFlusher): (() => void) => {
  flushers.add(flush);
  return () => {
    flushers.delete(flush);
  };
};

/**
 * Speichert sofort alles Ungespeicherte; aufrufen vor `meetings_stop`. Liefert `true`, wenn alle
 * Notizbloecke gesichert sind, sonst `false`. Entwuerfe bleiben fuer weitere Versuche erhalten.
 * Wirft nicht; auch waehrend des Wartens neu erfasste Aenderungen zaehlen mit.
 * Nach der Gesamtfrist weiter im Hintergrund sichern; Schreibsperren bleiben bestehen.
 */
export const flushAllNotes = async (timeoutMs = 3000): Promise<boolean> => {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const deadline = new Promise<boolean>((resolve) => {
    timer = setTimeout(() => resolve(false), timeoutMs);
  });
  const saving = Promise.all(
    [...flushers].map((f) => Promise.resolve().then(f).catch(() => false)),
  ).then(
    (results) =>
      results.every(Boolean) &&
      [...drafts.values()].every((d) => !d.dirty && !d.inflight),
  );
  try {
    return await Promise.race([saving, deadline]);
  } finally {
    if (timer !== null) clearTimeout(timer);
  }
};

interface NotesDraft {
  id: string;
  blocks: NoteBlock[];
  revision: number;
  savedJson: string;
  dirty: boolean;
  status: NotesSaveStatus;
  conflicts: number;
  loaded: boolean;
  loadError: string | null;
  loading: Promise<void> | null;
  inflight: Promise<boolean> | null;
  timer: ReturnType<typeof setTimeout> | null;
  retryDelayMs: number;
  missingCheckAfter: number | null;
  listeners: Set<() => void>;
  discarded: boolean;
  flush: NotesFlusher;
  unregister: () => void;
}

// Nur lebende Editoren und unbestaetigte Entwuerfe bleiben im Register. Kein
// Groessen-LRU: gerade ein zu grosser Entwurf darf nicht still verloren gehen.
const drafts = new Map<string, NotesDraft>();
const notify = (d: NotesDraft) => d.listeners.forEach((listener) => listener());
const clearTimer = (d: NotesDraft) => {
  if (d.timer !== null) clearTimeout(d.timer);
  d.timer = null;
};
const release = (d: NotesDraft) => {
  if (d.listeners.size || d.dirty || d.inflight || d.loading) return;
  clearTimer(d);
  if (drafts.get(d.id) === d) drafts.delete(d.id);
  d.unregister();
};

/** Nach bestaetigtem Loeschen bzw. ausdruecklichem Verwerfen eigener Notizen. */
export const discardNotesDraft = (meetingId: string): void => {
  const d = drafts.get(meetingId);
  if (!d) return;
  d.discarded = true;
  d.dirty = false;
  d.blocks = [];
  d.savedJson = "[]";
  clearTimer(d);
  drafts.delete(meetingId);
  d.unregister();
  notify(d);
};

const scheduleSave = (d: NotesDraft, delay: number) => {
  clearTimer(d);
  if (d.discarded) return;
  d.timer = setTimeout(() => {
    d.timer = null;
    void d.flush();
  }, delay);
};

/** Ein Speichervorgang; die umgebende Flush-Sperre faengt auch IPC-Ausnahmen. */
const saveOnce = async (d: NotesDraft): Promise<void> => {
  let snapshot = d.blocks;
  let toSave = blocksForSave(snapshot);
  let json = JSON.stringify(toSave);
  if (json === d.savedJson) {
    d.dirty = false;
    d.status = "saved";
    return;
  }
  if (notesTooLarge(snapshot)) {
    d.status = "tooLarge";
    return;
  }
  d.status = "saving";
  notify(d);
  let result = await commands.meetingNotesSave(d.id, toSave, d.revision);
  if (d.discarded) return;
  if (result.status === "error" && result.error === "revision_conflict") {
    const remote = await commands.meetingNotesGet(d.id);
    if (d.discarded) return;
    if (remote.status === "ok") {
      // Nach BEIDEN awaits den aktuellen lokalen Stand verwenden.
      snapshot = mergeConflict(remote.data.blocks, d.blocks);
      d.blocks = snapshot;
      d.revision = remote.data.revision;
      d.savedJson = JSON.stringify(blocksForSave(remote.data.blocks));
      d.conflicts++;
      notify(d);
      if (notesTooLarge(snapshot)) {
        d.status = "tooLarge";
        return;
      }
      toSave = blocksForSave(snapshot);
      json = JSON.stringify(toSave);
      result = await commands.meetingNotesSave(d.id, toSave, d.revision);
      if (d.discarded) return;
    }
  }
  if (result.status === "ok") {
    d.missingCheckAfter = null;
    d.revision = result.data;
    d.savedJson = json;
    // Nur genau die bestaetigte Fassung darf dirty zuruecknehmen.
    d.dirty = d.blocks !== snapshot;
    d.status = d.dirty ? "dirty" : "saved";
  } else {
    d.status = "error";
  }
};

const flushDraft = (d: NotesDraft): Promise<boolean> => {
  clearTimer(d);
  if (d.inflight) return d.inflight;
  const run = (async () => {
    try {
      if (d.discarded) return true;
      if (
        d.dirty && !d.listeners.size &&
        (notesTooLarge(d.blocks) || d.status === "error") &&
        (d.missingCheckAfter === null || Date.now() >= d.missingCheckAfter)
      ) {
        // Nur verwaiste, nicht sicherbare Entwuerfe pruefen. Zwei Null-Antworten
        // im Wiederholungsabstand bestaetigen die Loeschung; Lesefehler nicht.
        const checkAfter = d.missingCheckAfter;
        d.missingCheckAfter = null;
        const meeting = await commands.meetingsGet(d.id);
        if (d.discarded) return true;
        // Ein waehrend des IPC wieder geoeffneter Editor behaelt seinen Text.
        if (!d.listeners.size) {
          if (meeting.status === "ok" && meeting.data === null) {
            if (checkAfter !== null) {
              discardNotesDraft(d.id);
              return true;
            }
            d.missingCheckAfter = Date.now() + d.retryDelayMs;
          }
          if (meeting.status === "error") {
            d.status = "error";
            return false;
          }
        }
      }
      if (d.loadError && !d.loading) void loadDraft(d);
      if (d.loading) await d.loading;
      // Hoechstens ein Speichern zugleich; neue Eingaben begrenzt nachholen.
      for (let round = 0; round < 3; round++) {
        if (d.discarded || !d.dirty) return true;
        // Ohne geladene Basisrevision keine fremden Notizen ueberschreiben.
        if (d.loadError) return false;
        const before = d.blocks;
        await saveOnce(d);
        if (d.blocks === before) return !d.dirty;
      }
      return !d.dirty;
    } catch {
      d.status = "error";
      return false;
    }
  })();
  d.inflight = run;
  void run.then(() => {
    if (d.inflight === run) d.inflight = null;
    notify(d);
    // Auch ohne Editor weiter versuchen, bei zu grossen Entwuerfen nur pruefen.
    if (d.dirty) {
      scheduleSave(d, d.retryDelayMs);
      d.retryDelayMs = Math.min(d.retryDelayMs * 2, 60_000);
    } else d.retryDelayMs = 5000;
    release(d);
  });
  return run;
};

const loadDraft = (d: NotesDraft): Promise<void> => {
  const run = (async () => {
    try {
      const result = await commands.meetingNotesGet(d.id);
      if (d.discarded) return;
      if (result.status === "ok") {
        d.revision = result.data.revision;
        d.savedJson = JSON.stringify(blocksForSave(result.data.blocks));
        if (!d.dirty) d.blocks = result.data.blocks;
        else {
          d.blocks = mergeConflict(result.data.blocks, d.blocks);
          d.conflicts++;
        }
        d.loadError = null;
      } else {
        d.loadError = result.error;
      }
    } catch {
      d.loadError = "store_failed";
    }
    d.loaded = true;
  })();
  d.loading = run;
  void run.then(() => {
    if (d.loading === run) d.loading = null;
    notify(d);
    release(d);
  });
  return run;
};

const openDraft = (id: string): NotesDraft => {
  const existing = drafts.get(id);
  // Ungespeichertes behaelt seine Basisrevision: ein fremder neuerer Stand
  // muss beim Speichern durch die Konfliktbehandlung gehen.
  if (existing) return existing;
  const d: NotesDraft = {
    id,
    blocks: [],
    revision: 0,
    savedJson: "[]",
    dirty: false,
    status: "idle",
    conflicts: 0,
    loaded: false,
    loadError: null,
    loading: null,
    inflight: null,
    timer: null,
    retryDelayMs: 5000,
    missingCheckAfter: null,
    listeners: new Set(),
    discarded: false,
    flush: () => flushDraft(d),
    unregister: () => {},
  };
  drafts.set(id, d);
  d.unregister = registerNotesFlusher(d.flush);
  void loadDraft(d);
  return d;
};

/** Audioposition nur fuer genau diese aufgenommene Besprechung. */
export const recordingStamper =
  (meetingId: string) => async (): Promise<number | null> => {
    const result = await commands.meetingsRecordingPosition();
    if (result.status === "ok" && result.data?.meeting_id === meetingId) {
      return result.data.position_ms;
    }
    return null;
  };

/** Editor-Anbindung; Speicherauftraege gehoeren der Besprechung, nicht dem Mount. */
export const useNotesAutosave = (
  meetingId: string | null,
  debounceMs: number = AUTOSAVE_DEBOUNCE_MS,
): NotesAutosave => {
  const draftRef = useRef<NotesDraft | null>(null);
  const [, rerender] = useState(0);
  useEffect(() => {
    const d = meetingId ? openDraft(meetingId) : null;
    draftRef.current = d;
    const update = () => rerender((n) => n + 1);
    d?.listeners.add(update);
    if (d) d.missingCheckAfter = null;
    if (d?.loadError && !d.loading) void loadDraft(d);
    update();
    return () => {
      d?.listeners.delete(update);
      if (d) void d.flush();
      draftRef.current = null;
    };
  }, [meetingId]);

  const flush = useCallback(async (): Promise<boolean> => {
    return draftRef.current ? draftRef.current.flush() : true;
  }, []);
  const setBlocks = useCallback(
    (next: BlocksUpdater) => {
      const d = draftRef.current;
      if (!d || d.discarded) return;
      const value = typeof next === "function" ? next(d.blocks) : next;
      if (value === d.blocks) return;
      d.blocks = value;
      d.dirty = true;
      d.status = "dirty";
      d.retryDelayMs = 5000;
      d.missingCheckAfter = null;
      notify(d);
      scheduleSave(d, debounceMs);
    },
    [debounceMs],
  );

  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === "hidden") void flushAllNotes();
    };
    const onPageHide = () => void flushAllNotes();
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", onPageHide);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", onPageHide);
    };
  }, []);

  const d = draftRef.current?.id === meetingId ? draftRef.current : null;
  return {
    blocks: d?.id === meetingId ? d.blocks : [],
    setBlocks,
    status: d?.status ?? "idle",
    conflicts: d?.conflicts ?? 0,
    loaded: meetingId === null || (d?.id === meetingId && d.loaded) || false,
    loadError: d?.loadError ?? null,
    flush,
  };
};
