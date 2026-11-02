import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type LibraryBook } from "@/bindings";
import { CHANGED_EVENT, HYDRATED_EVENT } from "@/lib/library/progress";
import { transcriptToText } from "@/lib/library/books";
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
    async (bookId: string, meetingId: string) => {
      if (finishing.current.has(meetingId)) return;
      finishing.current.add(meetingId);
      let done = false;
      let pageId: string | null = null;
      try {
        const book = await freshBook(bookId);
        // Schon übernommen oder inzwischen ein anderer Auftrag: nichts tun.
        if (!book || book.text || book.audio?.transcript_job !== meetingId) {
          done = true;
          return;
        }
        const segments = await commands.meetingsGetSegments(meetingId);
        if (segments.status !== "ok") throw new Error(segments.error);
        const text = transcriptToText(segments.data);
        if (!text.trim()) throw new Error("Das Transkript ist leer.");
        const author = book.authors.length ? `\n\nvon ${book.authors.join(", ")}` : "";
        const created = await commands.pagesCreate(book.title);
        if (created.status !== "ok") throw new Error(created.error);
        pageId = created.data.id;
        const stored = await commands.pageStateSave(
          pageId,
          JSON.stringify({
            text: `# ${book.title}${author}\n\n${text}`,
            summary: "",
            sourceUrl: "",
            tab: "original",
          }),
        );
        if (stored.status !== "ok") throw new Error(stored.error);
        // Unmittelbar vor dem Schreiben noch einmal frisch lesen.
        const latest = (await freshBook(bookId)) ?? book;
        const saved = await commands.librarySave({
          ...latest,
          text: {
            page_id: pageId,
            origin: "transcript",
            source_name: latest.audio?.file ?? null,
          },
          audio: latest.audio ? { ...latest.audio, transcript_job: null } : null,
        });
        if (saved.status !== "ok") throw new Error(saved.error);
        pageId = null;
        done = true;
        // Erst jetzt, wo der Text sicher am Buch hängt, die Hilfsaufnahme weg.
        await commands.meetingsDelete(meetingId);
        await reload();
      } catch (e) {
        if (pageId) await commands.pagesDelete(pageId);
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        // Erfolgreich übernommene Jobs bleiben gesperrt; fehlgeschlagene
        // dürfen erst nach einer Pause erneut versucht werden.
        if (!done)
          window.setTimeout(() => finishing.current.delete(meetingId), 60_000);
      }
    },
    [reload],
  );

  // Laufende Transkriptionen beobachten, solange es welche gibt.
  const pendingIds = (books ?? [])
    .map((b) => b.audio?.transcript_job)
    .filter((id): id is string => !!id)
    .join(",");
  useEffect(() => {
    if (!pendingIds) {
      setJobs({});
      return;
    }
    let stop = false;
    const poll = async () => {
      const result = await commands.meetingsList(0, 500);
      if (stop || result.status !== "ok") return;
      const byId = new Map((result.data ?? []).map((m) => [m.id, m]));
      const next: Record<string, TranscriptJob> = {};
      for (const book of books ?? []) {
        const id = book.audio?.transcript_job;
        if (!id) continue;
        const meeting = byId.get(id);
        const status = meeting?.status ?? "missing";
        next[id] = { bookId: book.id, status, progress: null };
        if (status === "ready" || status === "done") void finishTranscript(book.id, id);
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
