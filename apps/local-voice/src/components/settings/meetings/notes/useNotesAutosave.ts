import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type NoteBlock } from "@/bindings";
import {
  AUTOSAVE_DEBOUNCE_MS,
  blocksForSave,
  mergeConflict,
  notesTooLarge,
} from "@/lib/meetingNotes";

/**
 * Autosave des Notizblocks (M1, P1c): entprellt, absturzsicher, mit
 * optimistischer Sperre.
 *
 * - Jede Aenderung startet die Entprellzeit neu; gespeichert wird der ganze
 *   Block-Satz (`meeting_notes_save`) gegen die zuletzt bekannte Revision.
 * - Sofort gespeichert wird bei Blur, `visibilitychange` (Fenster verdeckt),
 *   `pagehide`, beim Ausblenden der Komponente und ueber `flushAllNotes()`
 *   (z. B. vor `meetings_stop`). Verlust bei einem Absturz: hoechstens die
 *   Entprellzeit.
 * - Ein Speichern laeuft nie parallel zu einem zweiten; Aenderungen waehrend
 *   des Speicherns werden danach nachgeholt.
 * - `revision_conflict` (zweites Fenster): gespeicherten Stand laden, lokale
 *   Bloecke als Kopie anhaengen, erneut speichern. Nichts geht still verloren.
 * - Ueber 200 KB wird nicht gespeichert, sondern `tooLarge` gemeldet.
 */

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
  flush: () => Promise<void>;
}

// Alle laufenden Notizbloecke der Seite: `flushAllNotes()` wartet auf jeden.
const flushers = new Set<() => Promise<void>>();

/** Speichert sofort alles Ungespeicherte; aufrufen vor `meetings_stop`. */
export const flushAllNotes = async (): Promise<void> => {
  await Promise.all([...flushers].map((f) => f().catch(() => undefined)));
};

/**
 * Liefert die Audioposition (ms) fuer einen neuen Block, solange genau diese
 * Besprechung aufgenommen wird; sonst `null` (Block bleibt ohne Zeitstempel).
 */
export const recordingStamper =
  (meetingId: string) => async (): Promise<number | null> => {
    const result = await commands.meetingsRecordingPosition();
    if (
      result.status === "ok" &&
      result.data &&
      result.data.meeting_id === meetingId
    ) {
      return result.data.position_ms;
    }
    return null;
  };

export const useNotesAutosave = (
  meetingId: string | null,
  debounceMs: number = AUTOSAVE_DEBOUNCE_MS,
): NotesAutosave => {
  const [blocks, setBlocksState] = useState<NoteBlock[]>([]);
  const [status, setStatus] = useState<NotesSaveStatus>("idle");
  const [conflicts, setConflicts] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const blocksRef = useRef<NoteBlock[]>([]);
  const revisionRef = useRef(0);
  // JSON des zuletzt gespeicherten (gefilterten) Standes: gleiche Fassung = kein Aufruf.
  const savedJsonRef = useRef("[]");
  const dirtyRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inflightRef = useRef<Promise<void> | null>(null);
  const mountedRef = useRef(true);
  // Erst im Effekt umgestellt (nicht beim Rendern): das Aufraeumen des alten
  // Effekts speichert noch unter der alten Besprechungs-ID.
  const meetingRef = useRef<string | null>(null);

  const safeStatus = useCallback((s: NotesSaveStatus) => {
    if (mountedRef.current) setStatus(s);
  }, []);

  const clearTimer = () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  /** Ein Speichervorgang (ohne Schleife); wirft nicht. */
  const saveOnce = useCallback(async (): Promise<void> => {
    const id = meetingRef.current;
    if (!id || !dirtyRef.current) return;
    const snapshot = blocksRef.current;
    const toSave = blocksForSave(snapshot);
    const json = JSON.stringify(toSave);
    if (json === savedJsonRef.current) {
      dirtyRef.current = false;
      safeStatus("saved");
      return;
    }
    if (notesTooLarge(snapshot)) {
      safeStatus("tooLarge");
      return;
    }
    safeStatus("saving");
    let result = await commands.meetingNotesSave(
      id,
      toSave,
      revisionRef.current,
    );
    // Die Besprechung wurde waehrend des Speicherns gewechselt: das Ergebnis
    // gehoert dem alten Notizblock und darf den neuen nicht beruehren.
    if (meetingRef.current !== id) return;
    if (result.status === "error" && result.error === "revision_conflict") {
      const remote = await commands.meetingNotesGet(id);
      if (meetingRef.current !== id) return;
      if (remote.status === "ok") {
        const merged = mergeConflict(remote.data.blocks, snapshot);
        revisionRef.current = remote.data.revision;
        blocksRef.current = merged;
        if (mountedRef.current) {
          setBlocksState(merged);
          setConflicts((n) => n + 1);
        }
        result = await commands.meetingNotesSave(
          id,
          blocksForSave(merged),
          revisionRef.current,
        );
        if (meetingRef.current !== id) return;
        if (result.status === "ok") {
          savedJsonRef.current = JSON.stringify(blocksForSave(merged));
          revisionRef.current = result.data;
          dirtyRef.current = false;
          safeStatus("saved");
        } else {
          safeStatus("error");
        }
        return;
      }
    }
    if (result.status === "ok") {
      revisionRef.current = result.data;
      savedJsonRef.current = json;
      // Wurde waehrend des Speicherns weitergetippt, bleibt der Stand ungespeichert.
      if (blocksRef.current === snapshot) {
        dirtyRef.current = false;
        safeStatus("saved");
      } else {
        safeStatus("dirty");
      }
    } else {
      safeStatus("error");
    }
  }, [safeStatus]);

  const flush = useCallback(async (): Promise<void> => {
    clearTimer();
    // Hoechstens ein Speichern zugleich; wurde waehrend des Speicherns
    // weitergetippt, wird der neue Stand nachgeholt (begrenzt, kein Endlosloop).
    for (let round = 0; round < 3; round++) {
      if (inflightRef.current) await inflightRef.current;
      if (!dirtyRef.current) return;
      const before = blocksRef.current;
      const run = saveOnce();
      inflightRef.current = run;
      await run;
      if (inflightRef.current === run) inflightRef.current = null;
      if (blocksRef.current === before) return;
    }
  }, [saveOnce]);

  const scheduleSave = useCallback(() => {
    clearTimer();
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void flush();
    }, debounceMs);
  }, [debounceMs, flush]);

  const setBlocks = useCallback(
    (next: BlocksUpdater) => {
      const value = typeof next === "function" ? next(blocksRef.current) : next;
      if (value === blocksRef.current) return;
      blocksRef.current = value;
      dirtyRef.current = true;
      setBlocksState(value);
      setStatus("dirty");
      scheduleSave();
    },
    [scheduleSave],
  );

  // Laden beim Oeffnen bzw. Wechsel der Besprechung.
  useEffect(() => {
    meetingRef.current = meetingId;
    let cancelled = false;
    setLoaded(false);
    setLoadError(null);
    blocksRef.current = [];
    setBlocksState([]);
    dirtyRef.current = false;
    revisionRef.current = 0;
    savedJsonRef.current = "[]";
    setStatus("idle");
    if (!meetingId) {
      setLoaded(true);
      return;
    }
    void commands.meetingNotesGet(meetingId).then((result) => {
      if (cancelled) return;
      if (result.status === "ok") {
        // Hat der Nutzer schon getippt, bevor die Antwort kam, bleibt sein Stand;
        // die Revision ist trotzdem die gespeicherte.
        revisionRef.current = result.data.revision;
        savedJsonRef.current = JSON.stringify(
          blocksForSave(result.data.blocks),
        );
        if (!dirtyRef.current) {
          blocksRef.current = result.data.blocks;
          setBlocksState(result.data.blocks);
        }
      } else {
        setLoadError(result.error);
      }
      setLoaded(true);
    });
    return () => {
      cancelled = true;
      // Besprechungswechsel oder Ausblenden: Ungespeichertes sofort wegschreiben
      // (der Aufruf geht synchron bis zum Backend-Aufruf, bevor der neue Effekt
      // die Refs zuruecksetzt).
      void flush();
    };
  }, [meetingId, flush]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Sofort speichern: Fenster verdeckt/geschlossen, Komponente verschwindet, Sammelaufruf.
  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === "hidden") void flush();
    };
    const onPageHide = () => void flush();
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", onPageHide);
    flushers.add(flush);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", onPageHide);
      flushers.delete(flush);
    };
  }, [flush]);

  return { blocks, setBlocks, status, conflicts, loaded, loadError, flush };
};
