import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type LibraryBook } from "@/bindings";
import { CHANGED_EVENT, HYDRATED_EVENT } from "@/lib/library/progress";
import {
  pendingJobs,
  textWithSync,
  tracksOf,
  transcriptParagraphs,
  type TranscriptSegment,
} from "@/lib/library/books";
import { toast } from "sonner";

async function freshBook(id: string): Promise<LibraryBook | null> {
  const result = await commands.libraryList();
  if (result.status !== "ok") return null;
  return (result.data ?? []).find((b) => b.id === id) ?? null;
}

const AUDIO_EXT = /\.(mp3|m4a|m4b|aac|wav|flac|ogg|opus|wma)$/i;
const POLL_MS = 5000;

export interface TranscriptJob {
  bookId: string;
  status: string;
  /** 0…1, wenn bekannt. */
  progress: number | null;
  /** Teile mit fertigem Transkript und Teile mit Auftrag insgesamt. */
  done: number;
  total: number;
}

const READY = new Set(["ready", "done"]);

/** Ein Status für alle Aufträge eines Buchs: schlechtester zuerst. */
export function aggregateStatus(statuses: string[]): string {
  if (statuses.some((s) => s === "failed")) return "failed";
  if (statuses.some((s) => s === "missing")) return "missing";
  if (statuses.length && statuses.every((s) => READY.has(s))) return "ready";
  if (statuses.some((s) => s === "processing" || s === "transcribing" || READY.has(s)))
    return "processing";
  return statuses[0] ?? "queued";
}

/**
 * Bücher, Fortschritt und laufende Transkriptionen der Bibliothek.
 *
 * Ist ein Hörbuch fertig transkribiert, wird der Text als Vorlesen-Seite
 * angelegt und als Textfassung an das Buch gehängt; die Hilfs-Besprechung der
 * Import-Warteschlange wird danach entfernt, damit das Hörbuch nicht unter
 * „Aufnahmen“ auftaucht.
 */
export function useLibrary() {
  const [books, setBooks] = useState<LibraryBook[] | null>(null);
  const [pageAudio, setPageAudio] = useState<Record<string, boolean>>({});
  const [jobs, setJobs] = useState<Record<string, TranscriptJob>>({});
  const [tick, setTick] = useState(0);
  /** Jobs in Übernahme oder übernommen: nie zweimal verarbeiten. */
  const finishing = useRef(new Set<string>());

  const reload = useCallback(async () => {
    const result = await commands.libraryList();
    if (result.status !== "ok") {
      setBooks([]);
      return;
    }
    // Robust gegen leere Antworten (z. B. Testmocks ohne Bibliothek).
    const list = result.data ?? [];
    setBooks(list);
    // Vertonte Dateien im Seitenordner = Audiofassung einer Textfassung.
    const entries = await Promise.all(
      list
        .filter((b) => b.text)
        .map(async (b) => {
          const files = await commands.pageFiles(b.text!.page_id);
          return [
            b.text!.page_id,
            files.status === "ok" &&
              (files.data ?? []).some((f) => AUDIO_EXT.test(f.name)),
          ] as const;
        }),
    );
    setPageAudio(Object.fromEntries(entries));
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Fortschritt geändert (Reader, Hörleiste, Abgleich beim Start): neu rechnen.
  useEffect(() => {
    const bump = () => setTick((n) => n + 1);
    window.addEventListener(CHANGED_EVENT, bump);
    window.addEventListener(HYDRATED_EVENT, bump);
    return () => {
      window.removeEventListener(CHANGED_EVENT, bump);
      window.removeEventListener(HYDRATED_EVENT, bump);
    };
  }, []);

  /**
   * Buch ändern: `update` bekommt den frisch gelesenen Stand, damit ein alter
   * Schnappschuss der Oberfläche keine inzwischen übernommene Textfassung,
   * kein Cover und keinen Job überschreibt.
   */
  const save = useCallback(
    async (
      id: string,
      update: (fresh: LibraryBook) => LibraryBook,
    ): Promise<LibraryBook | null> => {
      const fresh = await freshBook(id);
      if (!fresh) throw new Error("Buch nicht gefunden.");
      const result = await commands.librarySave(update(fresh));
      if (result.status !== "ok") throw new Error(result.error);
      await reload();
      return result.data;
    },
    [reload],
  );

  const finishTranscript = useCallback(
    async (bookId: string, durations: Map<string, number | null>) => {
      const start = await freshBook(bookId);
      if (!start?.audio || start.text) return;
      const jobs = pendingJobs(start);
      const guard = `${bookId}:${jobs.join(",")}`;
      if (finishing.current.has(guard)) return;
      finishing.current.add(guard);
      let done = false;
      let pageId: string | null = null;
      try {
        const book = (await freshBook(bookId)) ?? start;
        const tracks = tracksOf(book.audio);
        // Erst wenn jeder Teil seinen Auftrag hat und sie alle fertig sind.
        if (
          !book.audio ||
          book.text ||
          !tracks.length ||
          tracks.some((t) => !t.transcript_job)
        ) {
          done = true;
          return;
        }
        // Segmente aller Teile, jeder um die Dauer der vorigen Teile versetzt.
        const shifted: TranscriptSegment[] = [];
        const lengths: number[] = [];
        let offset = 0;
        for (const track of tracks) {
          const id = track.transcript_job!;
          const result = await commands.meetingsGetSegments(id);
          if (result.status !== "ok") throw new Error(result.error);
          const lastEnd = result.data.reduce((m, x) => Math.max(m, x.end_ms), 0);
          const length = track.duration_ms ?? durations.get(id) ?? lastEnd;
          lengths.push(length);
          for (const x of result.data)
            shifted.push({
              text: x.text,
              start_ms: x.start_ms + offset,
              end_ms: x.end_ms + offset,
            });
          offset += length;
        }
        const author = book.authors.length ? `\n\nvon ${book.authors.join(", ")}` : "";
        const { text, sync } = textWithSync(
          transcriptParagraphs(shifted),
          `# ${book.title}${author}\n\n`,
        );
        if (!sync.length) throw new Error("Das Transkript ist leer.");
        const created = await commands.pagesCreate(book.title);
        if (created.status !== "ok") throw new Error(created.error);
        pageId = created.data.id;
        const stored = await commands.pageStateSave(
          pageId,
          JSON.stringify({ text, summary: "", sourceUrl: "", tab: "original" }),
        );
        if (stored.status !== "ok") throw new Error(stored.error);
        // Unmittelbar vor dem Schreiben noch einmal frisch lesen.
        const latest = (await freshBook(bookId)) ?? book;
        if (!latest.audio) throw new Error("Das Hörbuch fehlt.");
        const multi = !!latest.audio.tracks?.length;
        const saved = await commands.librarySave({
          ...latest,
          text: {
            page_id: pageId,
            origin: "transcript",
            source_name: latest.audio.file ?? null,
          },
          audio: {
            ...latest.audio,
            transcript_job: null,
            sync,
            duration_ms: multi
              ? latest.audio.duration_ms
              : (latest.audio.duration_ms ?? lengths[0] ?? null),
            tracks: multi
              ? (latest.audio.tracks ?? []).map((t, i) => ({
                  ...t,
                  transcript_job: null,
                  duration_ms: t.duration_ms ?? lengths[i] ?? null,
                }))
              : [],
          },
        });
        if (saved.status !== "ok") throw new Error(saved.error);
        pageId = null;
        done = true;
        // Erst jetzt, wo der Text sicher am Buch hängt, die Hilfsaufnahmen weg.
        for (const track of tracks) await commands.meetingsDelete(track.transcript_job!);
        await reload();
      } catch (e) {
        if (pageId) await commands.pagesDelete(pageId);
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        // Erfolgreich übernommene Jobs bleiben gesperrt; fehlgeschlagene
        // dürfen erst nach einer Pause erneut versucht werden.
        if (!done)
          window.setTimeout(() => finishing.current.delete(guard), 60_000);
      }
    },
    [reload],
  );

  // Laufende Transkriptionen beobachten, solange es welche gibt.
  const pendingIds = (books ?? [])
    .flatMap((b) => pendingJobs(b))
    .join(",");
  useEffect(() => {
    if (!pendingIds) {
      setJobs({});
      return;
    }
    let stop = false;
    const poll = async () => {
      const result = await commands.meetingsList(0, 2000);
      if (stop || result.status !== "ok") return;
      const byId = new Map((result.data ?? []).map((m) => [m.id, m]));
      const durations = new Map(
        [...byId].map(([id, m]) => [id, m.duration_ms ?? null] as const),
      );
      const next: Record<string, TranscriptJob> = {};
      for (const book of books ?? []) {
        const ids = pendingJobs(book);
        if (!ids.length) continue;
        const statuses = ids.map((id) => byId.get(id)?.status ?? "missing");
        const status = aggregateStatus(statuses);
        next[book.id] = {
          bookId: book.id,
          status,
          progress: null,
          done: statuses.filter((s) => READY.has(s)).length,
          total: ids.length,
        };
        if (status === "ready") void finishTranscript(book.id, durations);
      }
      setJobs(next);
    };
    void poll();
    const handle = window.setInterval(() => void poll(), POLL_MS);
    return () => {
      stop = true;
      window.clearInterval(handle);
    };
  }, [pendingIds, books, finishTranscript]);

  return { books, pageAudio, jobs, reload, save, tick };
}
