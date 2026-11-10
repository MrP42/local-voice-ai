import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands, type LibraryBook } from "@/bindings";
import { CHANGED_EVENT, HYDRATED_EVENT } from "@/lib/library/progress";
import {
  assetUrl,
  pendingJobs,
  textWithSync,
  tracksOf,
  transcriptParagraphs,
  type TranscriptSegment,
} from "@/lib/library/books";
import { toast } from "sonner";
import { listen } from "@tauri-apps/api/event";
import i18n from "@/i18n";
import { probeDuration } from "@/lib/library/probe";
import { editionsOf } from "@/lib/library/editions";
import { resumeTranscription, transcribeTracks } from "@/lib/library/importFile";
import { useMeetingProgress } from "@/hooks/useMeetingJobs";
import type { ProgressMap } from "@/lib/meetingJobs";
import {
  aggregateStatus,
  isSectioned,
  jobKey,
  jobsOfTrack,
  mergeSections,
  RESTART_STATUSES,
} from "@/lib/library/sections";

/** Nach so vielen Fehlversuchen gilt die Übernahme als fehlgeschlagen. */
const MAX_FAILURES = 3;

/** Zwischenzeitlich geändertes Buch: die Übernahme wird still verworfen. */
class Superseded extends Error {}

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
  /** Aufträge (Teile oder Abschnitte) mit fertigem Transkript und Aufträge insgesamt. */
  done: number;
  total: number;
  /** Davon gerade in Arbeit (gleichzeitig) und fehlgeschlagen oder verschwunden. */
  running: number;
  failed: number;
  /** Aufträge ohne fertiges Transkript (für den anteiligen Fortschritt). */
  open: string[];
}

const READY = new Set(["ready", "done"]);

/**
 * Anteil der fertigen Transkription eines Buchs (0…1): fertige Aufträge zählen
 * ganz, laufende mit ihrem Stand aus `MeetingEvent::Progress` (Millisekunden
 * Audio), wartende gar nicht. Ohne Aufträge `null`.
 */
export function transcriptFraction(
  job: Pick<TranscriptJob, "done" | "total" | "open">,
  live: ProgressMap,
): number | null {
  if (!(job.total > 0)) return null;
  let partial = 0;
  // Nur offene Aufträge: fertige zählen schon in `done`.
  for (const id of job.open) {
    const p = live[id];
    if (!p || p.phase === "prepare") continue;
    if (p.phase !== "transcription") partial += 1;
    else if (p.total > 0) partial += Math.min(1, p.done / p.total);
  }
  return Math.min(1, (job.done + partial) / job.total);
}

export { aggregateStatus };

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
  /** Übernahmen, die gerade laufen (dürfen beim Wiederholen nicht freigegeben werden). */
  const inFlight = useRef(new Set<string>());
  /** Freigabe-Timer gescheiterter Übernahmen; „Erneut versuchen“ entwertet sie. */
  const releaseTimers = useRef(new Map<string, number>());

  /** Laufnummer: ein veraltetes Ergebnis überschreibt kein neueres. */
  const loadSeq = useRef(0);

  const reload = useCallback(async () => {
    const seq = ++loadSeq.current;
    const result = await commands.libraryList();
    if (seq !== loadSeq.current) return;
    if (result.status !== "ok") {
      setBooks([]);
      return;
    }
    // Robust gegen leere Antworten (z. B. Testmocks ohne Bibliothek).
    const list = result.data ?? [];
    setBooks(list);
    // Vertonte Dateien im Seitenordner = Audiofassung einer Textfassung.
    // Auch die Seiten der übrigen Sprachfassungen (lva#344).
    const pageIds = [
      ...new Set(
        list.flatMap((b) =>
          editionsOf(b).flatMap((e) => (e.text ? [e.text.page_id] : [])),
        ),
      ),
    ];
    const entries = await Promise.all(
      pageIds.map(async (pageId) => {
        const files = await commands.pageFiles(pageId);
        return [
          pageId,
          files.status === "ok" &&
            (files.data ?? []).some((f) => AUDIO_EXT.test(f.name)),
        ] as const;
      }),
    );
    if (seq !== loadSeq.current) return;
    setPageAudio(Object.fromEntries(entries));
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Geräte-Sync (lva#210): Bücher von anderen Geräten erscheinen sofort.
  useEffect(() => {
    const unlisten = listen("sync-changed", () => void reload());
    return () => {
      void unlisten.then((fn) => fn()).catch(() => {});
    };
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

  /** Fehlversuche je Buch; nach {@link MAX_FAILURES} wird nicht mehr versucht. */
  const failures = useRef(new Map<string, number>());
  const [gaveUp, setGaveUp] = useState<ReadonlySet<string>>(new Set());
  const gaveUpRef = useRef(gaveUp);
  gaveUpRef.current = gaveUp;
  /** Fehlende Teil-Aufträge wurden für dieses Buch schon einmal nachgestartet. */
  const restarted = useRef(new Set<string>());

  const giveUp = useCallback((bookId: string, message: string) => {
    setGaveUp((prev) => new Set(prev).add(bookId));
    toast.error(message);
  }, []);

  const finishTranscript = useCallback(
    async (bookId: string, durations: Map<string, number | null>) => {
      if (gaveUpRef.current.has(bookId)) return;
      const start = await freshBook(bookId);
      if (!start?.audio) return;
      const startTracks = tracksOf(start.audio);
      const withJob = startTracks.filter((t) => jobsOfTrack(t).length);
      if (!withJob.length) return;
      // Anhängen an ein Transkript: der Text wächst, sonst entsteht er neu.
      const appendTo = start.text?.origin === "transcript" ? start.text : null;
      if (start.text && !appendTo) return;
      // Fehlt einem Teil der Auftrag (z. B. Abbruch beim Starten), nachstarten
      // statt ewig zu warten; ein zweites Mal meldet es sich als Fehler.
      if (!appendTo && withJob.length < startTracks.length) {
        if (restarted.current.has(bookId)) {
          giveUp(bookId, i18n.t("library.job.missingParts"));
          return;
        }
        restarted.current.add(bookId);
        await transcribeTracks(bookId, (m) => toast.error(m));
        await reload();
        return;
      }
      const jobs = pendingJobs(start);
      const guard = `${bookId}:${jobs.join(",")}`;
      if (finishing.current.has(guard)) return;
      finishing.current.add(guard);
      inFlight.current.add(guard);
      let done = false;
      let created: string | null = null;
      let restore: { pageId: string; json: string } | null = null;
      try {
        const book = start;
        const tracks = startTracks;
        const evaluated = withJob.flatMap((t) =>
          jobsOfTrack(t).map((p) => ({ file: t.file, job: p.job })),
        );
        /** Datei → ausgewertete Aufträge; ändert sich einer, wird verworfen. */
        const evaluatedKeys = new Map(withJob.map((t) => [t.file, jobKey(t)] as const));
        const isEvaluated = (t: (typeof tracks)[number]) =>
          evaluatedKeys.get(t.file) === jobKey(t);
        const first = tracks.findIndex((t) => jobsOfTrack(t).length);
        const paths = book.audio_paths ?? [];
        // Dauer je Teil: gespeichert, aus der Datei, aus dem Auftrag; das Ende
        // des letzten Segments ist nur der letzte Rückfall und wird nicht gesichert.
        const lengths: number[] = [];
        const storable: (number | null)[] = [];
        for (let i = 0; i < tracks.length; i++) {
          const track = tracks[i];
          let ms: number | null = track.duration_ms ?? null;
          if (!(ms && ms > 0)) {
            const url = assetUrl(paths[i]);
            ms = url ? await probeDuration(url) : null;
          }
          if (!(ms && ms > 0) && !isSectioned(track) && track.transcript_job)
            ms = durations.get(track.transcript_job) ?? null;
          storable.push(ms && ms > 0 ? ms : null);
          lengths.push(ms && ms > 0 ? ms : 0);
        }
        // Segmente der neuen Teile, jeder um die Dauer der vorigen Teile versetzt.
        const shifted: TranscriptSegment[] = [];
        let offset = lengths.slice(0, appendTo ? first : 0).reduce((a, b) => a + b, 0);
        if (appendTo && lengths.slice(0, first).some((l) => !(l > 0)))
          throw new Error(i18n.t("library.job.unknownDuration"));
        for (let i = appendTo ? first : 0; i < tracks.length; i++) {
          const track = tracks[i];
          const trackJobs = jobsOfTrack(track);
          if (!trackJobs.length) continue;
          // Abschnitte einer langen Datei: jeder Auftrag liefert seinen Bereich,
          // zusammengeführt entsteht die Folge der ganzen Datei.
          const sections = [];
          for (const part of trackJobs) {
            const result = await commands.meetingsGetSegments(part.job);
            if (result.status !== "ok") throw new Error(result.error);
            sections.push({ part, segments: result.data });
          }
          const fileSegments = mergeSections(sections);
          if (!(lengths[i] > 0))
            lengths[i] = fileSegments.reduce((m, x) => Math.max(m, x.end_ms), 0);
          for (const x of fileSegments)
            shifted.push({
              text: x.text,
              start_ms: x.start_ms + offset,
              end_ms: x.end_ms + offset,
            });
          offset += lengths[i];
        }
        const paragraphs = transcriptParagraphs(shifted);
        let text: string;
        let sync: { chars: number; ms: number }[];
        let pageId: string;
        if (appendTo) {
          const loaded = await commands.pageStateLoad(appendTo.page_id);
          if (loaded.status !== "ok") throw new Error(loaded.error);
          const state = JSON.parse(loaded.data) as { text?: string };
          const old = (state.text ?? "").replace(/\s+$/, "");
          const grown = textWithSync(paragraphs, `${old}\n\n`);
          text = grown.text;
          sync = [...(book.audio?.sync ?? []), ...grown.sync];
          pageId = appendTo.page_id;
          if (!grown.sync.length) throw new Error(i18n.t("library.job.emptyTranscript"));
          restore = { pageId, json: loaded.data };
          const stored = await commands.pageStateSave(
            pageId,
            JSON.stringify({ ...state, text }),
          );
          if (stored.status !== "ok") throw new Error(stored.error);
        } else {
          const author = book.authors.length ? `\n\nvon ${book.authors.join(", ")}` : "";
          ({ text, sync } = textWithSync(paragraphs, `# ${book.title}${author}\n\n`));
          if (!sync.length) throw new Error(i18n.t("library.job.emptyTranscript"));
          const page = await commands.pagesCreate(book.title);
          if (page.status !== "ok") throw new Error(page.error);
          pageId = page.data.id;
          created = pageId;
          const stored = await commands.pageStateSave(
            pageId,
            JSON.stringify({ text, summary: "", sourceUrl: "", tab: "original" }),
          );
          if (stored.status !== "ok") throw new Error(stored.error);
        }
        // Unmittelbar vor dem Schreiben noch einmal frisch lesen: hat sich
        // inzwischen etwas geändert (Text, Teile, Aufträge), wird verworfen.
        // Inzwischen entfernt (Papierkorb): nichts schreiben.
        const fresh = await freshBook(bookId);
        if (!fresh) {
          done = true;
          throw new Superseded();
        }
        const latest = fresh;
        const latestTracks = tracksOf(latest.audio);
        const unchanged =
          !!latest.audio &&
          [...evaluatedKeys].every(([file, key]) =>
            latestTracks.some((t) => t.file === file && jobKey(t) === key),
          ) &&
          (appendTo
            ? latest.text?.page_id === appendTo.page_id
            : !latest.text);
        if (!unchanged || !latest.audio) {
          done = true;
          if (!appendTo && latest.text && latest.audio) {
            // Ein anderer Weg hat inzwischen einen Text angelegt: unsere
            // ausgewerteten Aufträge sind überflüssig.
            const cleared = latestTracks.map((t) =>
              isEvaluated(t) ? { ...t, transcript_job: null, transcript_jobs: [] } : t,
            );
            const saved = await commands.librarySave({
              ...latest,
              audio: { ...latest.audio, tracks: latest.audio.tracks?.length ? cleared : [] },
            });
            if (saved.status === "ok")
              for (const e of evaluated) await commands.meetingsDelete(e.job);
          }
          throw new Superseded();
        }
        const multi = !!latest.audio.tracks?.length;
        const saved = await commands.librarySave({
          ...latest,
          text: appendTo
            ? latest.text
            : { page_id: pageId, origin: "transcript", source_name: latest.audio.file ?? null },
          audio: {
            ...latest.audio,
            transcript_job: null,
            sync,
            duration_ms: multi
              ? latest.audio.duration_ms
              : (latest.audio.duration_ms ?? storable[0] ?? null),
            tracks: multi
              ? latestTracks.map((t) => {
                  const i = tracks.findIndex((x) => x.file === t.file);
                  const keep = t.duration_ms ?? storable[i] ?? null;
                  return isEvaluated(t)
                    ? { ...t, transcript_job: null, transcript_jobs: [], duration_ms: keep }
                    : { ...t, duration_ms: keep };
                })
              : [],
          },
        });
        if (saved.status !== "ok") throw new Error(saved.error);
        created = null;
        restore = null;
        done = true;
        // Erst jetzt, wo der Text sicher am Buch hängt, die ausgewerteten
        // Hilfsaufnahmen weg.
        for (const e of evaluated) await commands.meetingsDelete(e.job);
        failures.current.delete(bookId);
        await reload();
      } catch (e) {
        if (created) await commands.pagesDelete(created);
        if (restore) await commands.pageStateSave(restore.pageId, restore.json);
        if (e instanceof Superseded) {
          await reload();
        } else {
          const n = (failures.current.get(bookId) ?? 0) + 1;
          failures.current.set(bookId, n);
          const message = e instanceof Error ? e.message : String(e);
          if (n >= MAX_FAILURES) {
            done = true;
            giveUp(bookId, message);
          }
        }
      } finally {
        // Erfolgreich übernommene Jobs bleiben gesperrt; fehlgeschlagene
        // dürfen erst nach einer Pause erneut versucht werden.
        inFlight.current.delete(guard);
        if (!done) {
          const old = releaseTimers.current.get(guard);
          if (old !== undefined) window.clearTimeout(old);
          const timer = window.setTimeout(() => {
            if (releaseTimers.current.get(guard) !== timer) return;
            releaseTimers.current.delete(guard);
            // Ein inzwischen neu gestarteter Versuch behält seine Sperre.
            if (!inFlight.current.has(guard)) finishing.current.delete(guard);
          }, 60_000);
          releaseTimers.current.set(guard, timer);
        }
      }
    },
    [reload, giveUp],
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
    let handle = 0;
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
        const status = gaveUpRef.current.has(book.id) ? "failed" : aggregateStatus(statuses);
        next[book.id] = {
          bookId: book.id,
          status,
          progress: null,
          done: statuses.filter((s) => READY.has(s)).length,
          total: ids.length,
          running: statuses.filter((s) => s === "processing" || s === "transcribing").length,
          failed: statuses.filter((s) => RESTART_STATUSES.has(s)).length,
          open: ids.filter((_, i) => !READY.has(statuses[i])),
        };
        if (status === "ready") void finishTranscript(book.id, durations);
      }
      setJobs(next);
      // Nichts läuft mehr (alles fehlgeschlagen oder verschwunden): nicht weiter abfragen.
      const live = Object.values(next);
      if (live.length && live.every((j) => j.status === "failed" || j.status === "missing")) {
        stop = true;
        window.clearInterval(handle);
      }
    };
    void poll();
    handle = window.setInterval(() => void poll(), POLL_MS);
    return () => {
      stop = true;
      window.clearInterval(handle);
    };
  }, [pendingIds, books, finishTranscript, gaveUp]);

  /**
   * Gescheiterte Transkription erneut anstoßen: Fehlversuche vergessen,
   * fehlgeschlagene oder verschwundene Aufträge neu starten. Waren alle
   * Aufträge fertig und nur die Übernahme scheiterte, versucht die nächste
   * Abfrage sie erneut.
   */
  const retryTranscript = useCallback(
    async (bookId: string) => {
      failures.current.delete(bookId);
      restarted.current.delete(bookId);
      for (const [guard, timer] of [...releaseTimers.current])
        if (guard.startsWith(`${bookId}:`)) {
          window.clearTimeout(timer);
          releaseTimers.current.delete(guard);
        }
      for (const guard of [...finishing.current])
        if (guard.startsWith(`${bookId}:`) && !inFlight.current.has(guard))
          finishing.current.delete(guard);
      setGaveUp((prev) => {
        if (!prev.has(bookId)) return prev;
        const next = new Set(prev);
        next.delete(bookId);
        return next;
      });
      await resumeTranscription(bookId, (m) => toast.error(m));
      await reload();
    },
    [reload],
  );

  // Fortschritt je Buch aus dem Stand der einzelnen Aufträge.
  const live = useMeetingProgress();
  const withProgress = useMemo(() => {
    const out: Record<string, TranscriptJob> = {};
    for (const [id, job] of Object.entries(jobs)) {
      out[id] = { ...job, progress: transcriptFraction(job, live) };
    }
    return out;
  }, [jobs, live]);

  return {
    books,
    pageAudio,
    jobs: withProgress,
    reload,
    save,
    tick,
    retryTranscript,
  };
}
