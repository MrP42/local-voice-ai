import { commands, type LibraryBook, type MetadataCandidate } from "@/bindings";
import i18n from "@/i18n";
import { patchFrom } from "@/components/library/MetadataPicker";
import {
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
      // Ein Hörbuch, keine Gesprächsaufnahme: keine Einwilligung nötig.
      const job = await commands.meetingsImportFile(book.audio_path, true, null);
      if (job.status === "ok") {
        const withJob = await commands.librarySave({
          ...book,
          audio: { ...book.audio, transcript_job: job.data },
        });
        if (withJob.status === "ok") book = withJob.data;
        else {
          // Ohne Verweis am Buch fände niemand den Auftrag wieder.
          await commands.meetingsDelete(job.data);
          onWarning?.(withJob.error);
        }
      } else onWarning?.(job.error);
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

/**
 * Transkription für jeden Teil ohne Auftrag und ohne Text starten, ein
 * Hilfs-Meeting je Teil (die Warteschlange nimmt nur leere Besprechungen auf).
 * Die Aufträge landen am Teil. Gibt das gespeicherte Buch zurück.
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
  const paths = book.audio_paths ?? [];
  const jobs = new Map<number, string>();
  for (let i = from ?? 0; i < tracks.length; i++) {
    if (tracks[i].transcript_job || !paths[i]) continue;
    // Ein Hörbuch, keine Gesprächsaufnahme: keine Einwilligung nötig.
    const r = await commands.meetingsImportFile(paths[i], true, null);
    if (r.status === "ok") jobs.set(i, r.data);
    else {
      onWarning?.(r.error);
      break;
    }
  }
  if (!jobs.size) return book;
  const latest = (await freshBook(bookId)) ?? book;
  const withJobs = applyJobs(latest, jobs);
  const saved = await commands.librarySave(withJobs);
  if (saved.status === "ok") return saved.data;
  // Ohne Verweis am Buch fände niemand die Aufträge wieder.
  for (const id of jobs.values()) await commands.meetingsDelete(id);
  onWarning?.(saved.error);
  return latest;
}

/** Aufträge (Teilindex → Meeting) in die Teile eintragen. */
export function applyJobs(book: LibraryBook, jobs: Map<number, string>): LibraryBook {
  if (!book.audio) return book;
  const tracks = tracksOf(book.audio);
  // Altbestand mit einer Datei behält den Einzel-Auftrag an der Fassung.
  if (!book.audio.tracks?.length) {
    const job = jobs.get(0);
    return job ? { ...book, audio: { ...book.audio, transcript_job: job } } : book;
  }
  return {
    ...book,
    audio: {
      ...book.audio,
      tracks: tracks.map((t, i) =>
        jobs.has(i) ? { ...t, transcript_job: jobs.get(i)! } : t,
      ),
    },
  };
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
  if (append && before.text?.origin === "transcript") {
    const next = await transcribeTracks(book.id, onWarning, tracksOf(before.audio).length);
    if (next) return next;
  }
  return r.data;
}
