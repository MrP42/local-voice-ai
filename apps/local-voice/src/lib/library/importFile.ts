import {
  commands,
  type AudioTrack,
  type LibraryBook,
  type MetadataCandidate,
} from "@/bindings";
import i18n from "@/i18n";
import { patchFrom } from "@/components/library/MetadataPicker";
import {
  assetUrl,
  extensionOf,
  fileNameOf,
  guessFromFileName,
  guessFromText,
  kindOfFile,
  pendingJobs,
  tracksOf,
} from "./books";
import type { AudioGroup } from "./audioGroups";
import { audioProgressKey, removeProgress, textProgressKey } from "./progress";
import { probeDuration } from "./probe";
import {
  isSectioned,
  jobsOfTrack,
  planSections,
  RESTART_STATUSES,
  type TrackJob,
} from "./sections";

/**
 * Datei in die Bibliothek aufnehmen (lva#152). Gemeinsamer Weg für den
 * Dialog „Hinzufügen“ und das Ablegen per Drag & Drop: ein Buch bleibt ein
 * Eintrag, eine neue Fassung wird an ein vorhandenes Buch gehängt.
 */

export interface PreparedFile {
  path: string;
  name: string;
  kind: "text" | "audio";
  /** Extrahierter Text (nur Textfassung). */
  text?: string;
  title: string;
  authors: string[];
}

export class ImportError extends Error {}

/** Meldung für Audible-Dateien (aax, aa, aaxc): geschützt, nie umgangen. */
export const drmMessage = (name: string) => i18n.t("library.drm", { name });

const fail = (r: { status: string; error?: string }) => {
  if (r.status === "error") throw new ImportError(r.error);
};

/** Datei lesen und Titel und Autor vermuten (Text aus dem Inhalt, sonst Name). */
export async function prepareFile(path: string): Promise<PreparedFile> {
  const kind = kindOfFile(path);
  if (kind === "drm") throw new ImportError(drmMessage(fileNameOf(path)));
  if (!kind) throw new ImportError(`unsupported:${extensionOf(path)}`);
  const name = path.split(/[\\/]/).pop() ?? path;
  let guess = guessFromFileName(path);
  let text: string | undefined;
  if (kind === "text") {
    const r = await commands.ttsExtractDocument(path);
    if (r.status === "error") throw new ImportError(r.error);
    text = r.data;
    guess = guessFromText(text) ?? guess;
  }
  return { path, name, kind, text, ...guess };
}

/** Hat das Buch schon eine Fassung dieser Art? */
export const hasKind = (book: LibraryBook, kind: PreparedFile["kind"]) =>
  kind === "text" ? !!book.text : !!book.audio;

/** Buch frisch aus dem Kern (nicht aus einem alten Oberflächenstand). */
export async function freshBook(id: string): Promise<LibraryBook | null> {
  const r = await commands.libraryList();
  return r.status === "ok" ? ((r.data ?? []).find((b) => b.id === id) ?? null) : null;
}

/** Vorhandenes Werk zu Titel/Autor/ISBN, sonst `null`. */
export async function findWork(
  title: string,
  authors: string[],
  isbn: string | null = null,
): Promise<LibraryBook | null> {
  const r = await commands.libraryFindDuplicate(title, authors, isbn);
  return r.status === "ok" ? r.data : null;
}

/** Leeres Buch zum Anlegen (die Kennung vergibt der Kern). */
export const emptyBook = (
  title: string,
  authors: string[],
  metadataSource: string,
): LibraryBook => ({
  id: "",
  title,
  subtitle: null,
  authors,
  language: null,
  isbn: null,
  publisher: null,
  publish_date: null,
  pages: null,
  description: null,
  metadata_source: metadataSource,
  cover_source: "generated",
  text: null,
  audio: null,
  added_ms: 0,
  finished: false,
  cover_path: null,
  audio_path: null,
  audio_paths: [],
});

export interface AddOptions {
  /** An dieses Buch anhängen (ersetzt eine vorhandene Fassung gleicher Art). */
  attachTo?: LibraryBook | null;
  title?: string;
  authors?: string[];
  /** Ausgabe aus Open Library (Metadaten und Cover). */
  candidate?: MetadataCandidate | null;
  /** Hörbuch ohne Text: lokal transkribieren (Vorgabe: ja). */
  transcribe?: boolean;
  /** Fehler, die das Aufnehmen nicht verhindern (z. B. Transkription). */
  onWarning?(message: string): void;
}

export async function addToLibrary(
  file: PreparedFile,
  options: AddOptions = {},
): Promise<{ book: LibraryBook; attached: boolean }> {
  const { attachTo, candidate, transcribe = true, onWarning } = options;
  const title = (options.title ?? file.title).trim() || file.name;
  const authors = options.authors ?? file.authors;
  const description = candidate?.work_key
    ? await commands
        .libraryDescription(candidate.work_key)
        .then((r) => (r.status === "ok" ? r.data : null))
    : null;
  // Anhängen immer an den aktuellen Stand: ein alter Schnappschuss würde eine
  // inzwischen angehängte Fassung wieder entfernen.
  const current = attachTo ? await freshBook(attachTo.id) : null;
  if (attachTo && !current) throw new ImportError("Buch nicht gefunden.");
  const base: LibraryBook =
    current ??
    emptyBook(
      title,
      authors,
      file.path.toLowerCase().endsWith(".epub") ? "epub" : "manual",
    );
  let book: LibraryBook = candidate
    ? { ...base, ...patchFrom(candidate, description) }
    : base;

  if (file.kind === "text" && file.text !== undefined) {
    const created = await commands.pagesCreate(book.title);
    fail(created);
    if (created.status !== "ok") throw new ImportError();
    const stored = await commands.pageStateSave(
      created.data.id,
      JSON.stringify({ text: file.text, summary: "", sourceUrl: "", tab: "original" }),
    );
    if (stored.status === "error") {
      await commands.pagesDelete(created.data.id);
      throw new ImportError(stored.error);
    }
    book = {
      ...book,
      text: { page_id: created.data.id, origin: "document", source_name: file.name },
    };
  }
  const saved = await commands.librarySave(book);
  if (saved.status !== "ok") {
    // Keine verwaiste Vorlesen-Seite zurücklassen.
    if (file.kind === "text" && book.text) await commands.pagesDelete(book.text.page_id);
    throw new ImportError(saved.error);
  }
  book = saved.data;

  if (file.kind === "audio") {
    const imported = await commands.libraryAudioImport(book.id, file.path);
    if (imported.status !== "ok") {
      // Neues Buch ohne jede Fassung wieder entfernen.
      if (!current) await commands.libraryDelete(book.id);
      throw new ImportError(imported.error);
    }
    book = imported.data;
    // Neue Hördatei: alte Aufträge, Transkript und Hörposition passen nicht mehr.
    if (current?.audio) await cleanupReplaced(current);
    if (transcribe && !book.text && book.audio_path && book.audio) {
      // Ein Hörbuch, keine Gesprächsaufnahme: keine Einwilligung nötig. Lange
      // Dateien laufen in Abschnitten (lva#208).
      const withJobs = await transcribeTracks(book.id, onWarning);
      if (withJobs) book = withJobs;
    }
  }

  if (candidate?.cover_id || (candidate?.isbn && book.cover_source === "generated")) {
    const r = await commands.libraryCoverFetch(
      book.id,
      candidate.cover_id ?? null,
      candidate.isbn ?? null,
    );
    if (r.status === "ok") book = r.data;
  } else if (
    book.cover_source === "generated" &&
    file.path.toLowerCase().endsWith(".epub")
  ) {
    const r = await commands.libraryCoverFromFile(book.id, file.path);
    if (r.status === "ok") book = r.data;
  } else if (file.kind === "audio") {
    book = await adoptAudioCover(book);
  }
  return { book, attached: !!current };
}

export interface GroupOptions {
  /** An dieses Buch anhängen. */
  attachTo?: LibraryBook | null;
  /** Neue Teile hinter die vorhandenen setzen statt sie zu ersetzen. */
  append?: boolean;
  title?: string;
  authors?: string[];
  candidate?: MetadataCandidate | null;
  transcribe?: boolean;
  onWarning?(message: string): void;
}

/** Aufträge, die für einen Teil gestartet wurden. */
export interface StartedTrack {
  /** Auftrag für den ganzen Teil (ohne Abschnitte). */
  job: string | null;
  /** Aufträge je Zeitabschnitt (lange Einzeldatei, lva#208). */
  parts: TrackJob[];
}

/** Dateipfade der Teile; ein Buch mit einer Datei kennt mindestens `audio_path`. */
const pathsOf = (book: LibraryBook): string[] =>
  book.audio_paths?.length ? book.audio_paths : book.audio_path ? [book.audio_path] : [];

/** Dauer des Teils in ms: gespeichert, sonst aus der Datei ermittelt. */
async function durationOfTrack(
  book: LibraryBook,
  index: number,
  path: string,
): Promise<number | null> {
  const track = tracksOf(book.audio)[index];
  const known = track?.duration_ms ?? (index === 0 ? book.audio?.duration_ms : null);
  if (known != null && known > 0) return known;
  const url = assetUrl(path);
  return url ? probeDuration(url) : null;
}

/**
 * Auftrag oder Aufträge für eine Datei starten: ein Hörbuch-Auftrag für den
 * ganzen Teil, bei `sections` (lange Einzeldatei) einer je Zeitabschnitt.
 * Scheitert einer, werden schon gestartete wieder entfernt.
 */
export async function startTrackJobs(
  path: string,
  durationMs: number | null,
  sections = true,
): Promise<StartedTrack | { error: string }> {
  const plan = sections ? planSections(durationMs) : [];
  if (!plan.length) {
    const r = await commands.meetingsImportAudiobookPart(path, null, null);
    return r.status === "ok" ? { job: r.data, parts: [] } : { error: r.error };
  }
  const parts: TrackJob[] = [];
  for (const s of plan) {
    const r = await commands.meetingsImportAudiobookPart(path, s.start_ms, s.end_ms);
    if (r.status !== "ok") {
      for (const p of parts) await commands.meetingsDelete(p.job);
      return { error: r.error };
    }
    parts.push({
      job: r.data,
      start_ms: s.start_ms,
      end_ms: s.end_ms,
      from_ms: s.from_ms,
      to_ms: s.to_ms,
    });
  }
  return { job: null, parts };
}

/** Alle Hilfs-Besprechungen, die ein Start angelegt hat. */
const startedIds = (started: Iterable<StartedTrack>): string[] =>
  [...started].flatMap((s) => (s.job ? [s.job] : s.parts.map((p) => p.job)));

/**
 * Transkription für jeden Teil ohne Auftrag und ohne Text starten, ein
 * Hilfs-Meeting je Teil (die Warteschlange nimmt nur leere Besprechungen auf);
 * hat das Hörbuch nur einen langen Teil, einen je Zeitabschnitt. Die Aufträge
 * landen am Teil. Gibt das gespeicherte Buch zurück.
 */
export async function transcribeTracks(
  bookId: string,
  onWarning?: (message: string) => void,
  /** Nur Teile ab diesem Index (angehängte Teile an ein Transkript). */
  from?: number,
): Promise<LibraryBook | null> {
  const book = await freshBook(bookId);
  if (!book?.audio) return book;
  // Ein fremder Text (EPUB …) bleibt; ein Transkript wächst nur um neue Teile.
  if (book.text && (book.text.origin !== "transcript" || from === undefined))
    return book;
  const tracks = tracksOf(book.audio);
  const paths = pathsOf(book);
  const jobs = new Map<number, StartedTrack>();
  for (let i = from ?? 0; i < tracks.length; i++) {
    if (jobsOfTrack(tracks[i]).length || !paths[i]) continue;
    // Nur ein einzelner Teil wird geteilt; mehrere Teile laufen schon parallel.
    const duration = tracks.length === 1 ? await durationOfTrack(book, i, paths[i]) : null;
    const r = await startTrackJobs(paths[i], duration);
    if ("error" in r) {
      onWarning?.(r.error);
      break;
    }
    jobs.set(i, r);
  }
  if (!jobs.size) return book;
  const latest = (await freshBook(bookId)) ?? book;
  const withJobs = applyJobs(latest, jobs);
  const saved = await commands.librarySave(withJobs);
  if (saved.status === "ok") return saved.data;
  // Ohne Verweis am Buch fände niemand die Aufträge wieder.
  for (const id of startedIds(jobs.values())) await commands.meetingsDelete(id);
  onWarning?.(saved.error);
  return latest;
}

/** Aufträge (Teilindex → gestartete Aufträge) in die Teile eintragen. */
export function applyJobs(
  book: LibraryBook,
  jobs: Map<number, StartedTrack>,
): LibraryBook {
  if (!book.audio) return book;
  const tracks = tracksOf(book.audio);
  const withJob = (t: AudioTrack, j: StartedTrack): AudioTrack => ({
    ...t,
    transcript_job: j.job,
    transcript_jobs: j.parts,
  });
  // Altbestand mit einer Datei behält den Einzel-Auftrag an der Fassung.
  if (!book.audio.tracks?.length) {
    const j = jobs.get(0);
    if (!j) return book;
    if (!j.parts.length)
      return { ...book, audio: { ...book.audio, transcript_job: j.job } };
    // Abschnitte brauchen die Teileliste: aus der einen Datei wird ein Teil.
    return {
      ...book,
      audio: { ...book.audio, transcript_job: null, tracks: [withJob(tracks[0], j)] },
    };
  }
  return {
    ...book,
    audio: {
      ...book.audio,
      tracks: tracks.map((t, i) => (jobs.has(i) ? withJob(t, jobs.get(i)!) : t)),
    },
  };
}

/** Das Buch mit neuen Aufträgen an den Teilen (Altbestand mit einer Datei bleibt es, wenn möglich). */
function withTracks(book: LibraryBook, tracks: AudioTrack[]): LibraryBook {
  if (!book.audio) return book;
  if (book.audio.tracks?.length) return { ...book, audio: { ...book.audio, tracks } };
  const [only] = tracks;
  if (!only) return book;
  if (isSectioned(only))
    return { ...book, audio: { ...book.audio, transcript_job: null, tracks: [only] } };
  return { ...book, audio: { ...book.audio, transcript_job: only.transcript_job ?? null } };
}

/**
 * Stand eines Auftrags, einzeln abgefragt: der Status, `null` bei wirklich
 * nicht gefunden, `undefined` bei einem Lesefehler (dann wird nichts neu gestartet).
 */
async function statusOfJob(job: string): Promise<string | null | undefined> {
  const r = await commands.meetingsGetStatus(job);
  return r.status === "ok" ? r.data : undefined;
}

/** Bücher, für die gerade ein Fortsetzen läuft (kein zweites parallel). */
const resuming = new Set<string>();

/**
 * Fehlende Teile transkribieren (lva#208): nur Aufträge, die fehlgeschlagen,
 * abgebrochen oder verschwunden sind, und Teile ohne Auftrag starten neu. Was
 * läuft oder fertig ist, bleibt unberührt; ein Abschnitt wird mit demselben
 * Bereich neu gestartet. Gibt das gespeicherte Buch zurück.
 */
export async function resumeTranscription(
  bookId: string,
  onWarning?: (message: string) => void,
): Promise<LibraryBook | null> {
  if (resuming.has(bookId)) return freshBook(bookId);
  resuming.add(bookId);
  try {
    return await resumeLocked(bookId, onWarning);
  } finally {
    resuming.delete(bookId);
  }
}

async function resumeLocked(
  bookId: string,
  onWarning?: (message: string) => void,
): Promise<LibraryBook | null> {
  const book = await freshBook(bookId);
  if (!book?.audio) return book;
  // Ein fremder Text bleibt; an ein Transkript hängt nur noch Fehlendes.
  if (book.text && book.text.origin !== "transcript") return book;
  // Jeder Auftrag einzeln: nur fehlgeschlagen, abgebrochen oder wirklich nicht
  // gefunden zählt. Was wartet oder läuft, und was sich nicht lesen lässt, bleibt.
  const ids = pendingJobs(book);
  const statuses = new Map(
    await Promise.all(ids.map(async (id) => [id, await statusOfJob(id)] as const)),
  );
  const needsRestart = (job: string) => {
    const status = statuses.get(job);
    return status === null || (typeof status === "string" && RESTART_STATUSES.has(status));
  };
  const tracks = tracksOf(book.audio);
  const paths = pathsOf(book);
  const created: string[] = [];
  const replaced: string[] = [];
  const next: AudioTrack[] = [];
  let failed: string | null = null;
  for (let i = 0; i < tracks.length; i++) {
    const track = tracks[i];
    const jobs = jobsOfTrack(track);
    const path = paths[i];
    if (failed || !path) {
      next.push(track);
      continue;
    }
    // Ohne Aufträge: nur ein Hörbuch ohne Text startet hier; mit Transkript
    // sind solche Teile schon im Text.
    if (!jobs.length) {
      if (book.text) {
        next.push(track);
        continue;
      }
      const duration = tracks.length === 1 ? await durationOfTrack(book, i, path) : null;
      const r = await startTrackJobs(path, duration);
      if ("error" in r) {
        failed = r.error;
        next.push(track);
        continue;
      }
      created.push(...startedIds([r]));
      next.push({ ...track, transcript_job: r.job, transcript_jobs: r.parts });
      continue;
    }
    if (!isSectioned(track)) {
      const job = jobs[0].job;
      if (!needsRestart(job)) {
        next.push(track);
        continue;
      }
      const duration = tracks.length === 1 ? await durationOfTrack(book, i, path) : null;
      const r = await startTrackJobs(path, duration);
      if ("error" in r) {
        failed = r.error;
        next.push(track);
        continue;
      }
      replaced.push(job);
      created.push(...startedIds([r]));
      next.push({ ...track, transcript_job: r.job, transcript_jobs: r.parts });
      continue;
    }
    // Abschnitte: jeder fehlende mit demselben Bereich neu.
    const parts: TrackJob[] = [];
    for (const part of jobs) {
      if (failed || !needsRestart(part.job)) {
        parts.push(part);
        continue;
      }
      const r = await commands.meetingsImportAudiobookPart(
        path,
        part.start_ms,
        part.end_ms ?? null,
      );
      if (r.status !== "ok") {
        failed = r.error;
        parts.push(part);
        continue;
      }
      replaced.push(part.job);
      created.push(r.data);
      parts.push({ ...part, job: r.data });
    }
    next.push({ ...track, transcript_jobs: parts });
  }
  if (failed) onWarning?.(failed);
  if (!created.length) return book;
  const latest = (await freshBook(bookId)) ?? book;
  const saved = await commands.librarySave(withTracks(latest, next));
  if (saved.status !== "ok") {
    // Ohne Verweis am Buch fände niemand die neuen Aufträge wieder.
    for (const id of created) await commands.meetingsDelete(id);
    onWarning?.(saved.error);
    return latest;
  }
  // Erst jetzt, wo das Buch auf die neuen Aufträge zeigt, die alten weg.
  for (const id of replaced) await commands.meetingsDelete(id);
  return saved.data;
}

/**
 * Mehrere Audiodateien als **ein** Hörbuch aufnehmen (lva#204): ein Buch mit
 * einer Teileliste statt eines Buchs je Datei. Eine einzelne Datei geht den
 * gewohnten Weg ({@link addToLibrary}).
 */
export async function addAudioGroup(
  group: Pick<AudioGroup, "paths" | "title" | "authors">,
  options: GroupOptions = {},
): Promise<{ book: LibraryBook; attached: boolean }> {
  const { attachTo, append = false, candidate, transcribe = true, onWarning } = options;
  const paths = group.paths;
  for (const path of paths) {
    const kind = kindOfFile(path);
    if (kind === "drm") throw new ImportError(drmMessage(fileNameOf(path)));
    if (kind !== "audio") throw new ImportError(`unsupported:${extensionOf(path)}`);
  }
  if (paths.length === 1 && !append) {
    const path = paths[0];
    return addToLibrary(
      {
        path,
        name: fileNameOf(path),
        kind: "audio",
        title: group.title,
        authors: group.authors,
      },
      { ...options, attachTo },
    );
  }
  const title = (options.title ?? group.title).trim() || fileNameOf(paths[0]);
  const authors = options.authors ?? group.authors;
  const current = attachTo ? await freshBook(attachTo.id) : null;
  if (attachTo && !current) throw new ImportError("Buch nicht gefunden.");
  const description = candidate?.work_key
    ? await commands
        .libraryDescription(candidate.work_key)
        .then((r) => (r.status === "ok" ? r.data : null))
    : null;
  let book: LibraryBook = current ?? emptyBook(title, authors, "manual");
  if (candidate) book = { ...book, ...patchFrom(candidate, description) };
  const saved = await commands.librarySave(book);
  if (saved.status !== "ok") throw new ImportError(saved.error);
  book = saved.data;

  const hadAudio = !!current?.audio;
  const imported = await commands.libraryAudioImportTracks(book.id, paths, append);
  if (imported.status !== "ok") {
    if (!current) await commands.libraryDelete(book.id);
    throw new ImportError(imported.error);
  }
  book = imported.data;
  // Ersetztes Hörbuch: alte Aufträge, Transkript und Hörposition passen nicht mehr.
  if (hadAudio && !append && current) await cleanupReplaced(current);

  if (transcribe) {
    const withJobs = await transcribeTracks(
      book.id,
      onWarning,
      // Angehängte Teile an ein Transkript: nur die neuen Teile transkribieren.
      append && current?.text?.origin === "transcript"
        ? tracksOf(current.audio).length
        : undefined,
    );
    if (withJobs) book = withJobs;
  }
  if (candidate?.cover_id || (candidate?.isbn && book.cover_source === "generated")) {
    const r = await commands.libraryCoverFetch(
      book.id,
      candidate.cover_id ?? null,
      candidate.isbn ?? null,
    );
    if (r.status === "ok") book = r.data;
  } else {
    book = await adoptAudioCover(book);
  }
  return { book, attached: !!current };
}

/**
 * Aufräumen, nachdem das Hörbuch eines Buchs **erfolgreich** ersetzt wurde:
 * laufende Aufträge der alten Teile löschen, die alte Hörposition entfernen und
 * einen Transkript-Text samt Seite verwerfen (der Kern hat ihn am Buch schon
 * gelöst), damit neu transkribiert wird.
 */
export async function cleanupReplaced(before: LibraryBook): Promise<void> {
  for (const id of pendingJobs(before)) await commands.meetingsDelete(id);
  removeProgress(audioProgressKey(before.id));
  if (before.text?.origin === "transcript") {
    removeProgress(textProgressKey(before.text.page_id));
    await commands.pagesDelete(before.text.page_id);
  }
}

/**
 * Hörbuch eines vorhandenen Buchs ersetzen oder Teile anhängen (Detailansicht).
 * Ersetzen räumt danach auf ({@link cleanupReplaced}); Anhängen an ein
 * Transkript startet die Transkription der neuen Teile, deren Text das
 * Transkript am Ende ergänzt.
 */
export async function importAudioParts(
  book: LibraryBook,
  paths: string[],
  append: boolean,
  onWarning?: (message: string) => void,
): Promise<LibraryBook> {
  const before = (await freshBook(book.id)) ?? book;
  const r =
    paths.length === 1 && !append
      ? await commands.libraryAudioImport(book.id, paths[0])
      : await commands.libraryAudioImportTracks(book.id, paths, append);
  if (r.status === "error") throw new ImportError(r.error);
  if (before.audio && !append) await cleanupReplaced(before);
  const withCover = await adoptAudioCover(r.data);
  if (append && before.text?.origin === "transcript") {
    const next = await transcribeTracks(book.id, onWarning, tracksOf(before.audio).length);
    if (next) return next;
  }
  return withCover;
}

/**
 * Bringt das Hörbuch ein eingebettetes Cover mit (MP3, M4B, FLAC) und hat
 * das Buch noch keins, wird es übernommen – lokal, ohne Netz. Ohne Bild
 * bleibt alles, wie es ist.
 */
export async function adoptAudioCover(book: LibraryBook): Promise<LibraryBook> {
  if (book.cover_source !== "generated" || !book.audio_path) return book;
  try {
    const r = await commands.libraryCoverFromAudio(book.id);
    return r.status === "ok" ? r.data : book;
  } catch {
    return book;
  }
}
