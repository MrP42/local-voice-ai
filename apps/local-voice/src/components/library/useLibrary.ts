import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type LibraryBook } from "@/bindings";
import { CHANGED_EVENT, HYDRATED_EVENT } from "@/lib/library/progress";
import { transcriptToText } from "@/lib/library/books";

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
  const finishing = useRef(new Set<string>());

  const reload = useCallback(async () => {
    const result = await commands.libraryList();
    if (result.status !== "ok") {
      setBooks([]);
      return;
    }
    setBooks(result.data);
    // Vertonte Dateien im Seitenordner = Audiofassung einer Textfassung.
    const entries = await Promise.all(
      result.data
        .filter((b) => b.text)
        .map(async (b) => {
          const files = await commands.pageFiles(b.text!.page_id);
          return [
            b.text!.page_id,
            files.status === "ok" &&
              files.data.some((f) => AUDIO_EXT.test(f.name)),
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

  const save = useCallback(
    async (book: LibraryBook): Promise<LibraryBook | null> => {
      const result = await commands.librarySave(book);
      if (result.status !== "ok") throw new Error(result.error);
      await reload();
      return result.data;
    },
    [reload],
  );

  const finishTranscript = useCallback(
    async (book: LibraryBook, meetingId: string) => {
      if (finishing.current.has(book.id)) return;
      finishing.current.add(book.id);
      try {
        const segments = await commands.meetingsGetSegments(meetingId);
        if (segments.status !== "ok") return;
        const text = transcriptToText(segments.data);
        if (!text.trim()) return;
        const author = book.authors.length ? `\n\nvon ${book.authors.join(", ")}` : "";
        const created = await commands.pagesCreate(book.title);
        if (created.status !== "ok") return;
        await commands.pageStateSave(
          created.data.id,
          JSON.stringify({
            text: `# ${book.title}${author}\n\n${text}`,
            summary: "",
            sourceUrl: "",
            tab: "original",
          }),
        );
        await commands.librarySave({
          ...book,
          text: {
            page_id: created.data.id,
            origin: "transcript",
            source_name: book.audio?.file ?? null,
          },
          audio: book.audio ? { ...book.audio, transcript_job: null } : null,
        });
        await commands.meetingsDelete(meetingId);
        await reload();
      } finally {
        finishing.current.delete(book.id);
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
      const byId = new Map(result.data.map((m) => [m.id, m]));
      const next: Record<string, TranscriptJob> = {};
      for (const book of books ?? []) {
        const id = book.audio?.transcript_job;
        if (!id) continue;
        const meeting = byId.get(id);
        const status = meeting?.status ?? "missing";
        next[id] = { bookId: book.id, status, progress: null };
        if (status === "ready" || status === "done") void finishTranscript(book, id);
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
