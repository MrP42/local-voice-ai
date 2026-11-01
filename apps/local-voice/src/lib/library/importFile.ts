import { commands, type LibraryBook, type MetadataCandidate } from "@/bindings";
import { patchFrom } from "@/components/library/MetadataPicker";
import {
  extensionOf,
  guessFromFileName,
  guessFromText,
  kindOfFile,
} from "./books";

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

const fail = (r: { status: string; error?: string }) => {
  if (r.status === "error") throw new ImportError(r.error);
};

/** Datei lesen und Titel und Autor vermuten (Text aus dem Inhalt, sonst Name). */
export async function prepareFile(path: string): Promise<PreparedFile> {
  const kind = kindOfFile(path);
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
  return r.status === "ok" ? (r.data.find((b) => b.id === id) ?? null) : null;
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
  const base: LibraryBook = current ?? {
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
    metadata_source: file.path.toLowerCase().endsWith(".epub") ? "epub" : "manual",
    cover_source: "generated",
    text: null,
    audio: null,
    added_ms: 0,
    finished: false,
    cover_path: null,
    audio_path: null,
  };
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
  fail(saved);
  if (saved.status !== "ok") throw new ImportError();
  book = saved.data;

  if (file.kind === "audio") {
    const imported = await commands.libraryAudioImport(book.id, file.path);
    fail(imported);
    if (imported.status !== "ok") throw new ImportError();
    book = imported.data;
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
