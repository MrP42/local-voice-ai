import { commands, type LibraryBook, type TrashEntry } from "@/bindings";
import { pendingJobs } from "./books";
import {
  audioProgressKey,
  flushProgress,
  removeProgress,
  textProgressKey,
} from "./progress";

/**
 * Entfernen (Codex-Abnahme lva#221): zwei klar getrennte Wege.
 * - **Papierkorb:** Der Kern verschiebt den Buchordner (Eintrag, Cover,
 *   Hörbuch-Kopie) nach `library/.papierkorb`. Wiederherstellbar über den
 *   Papierkorb der Bibliothek, 30 Tage lang. Der Text bleibt im Vorlesen.
 * - **Endgültig:** sofort und ohne Rückweg, samt Text bzw. Transkript im
 *   Vorlesen, laufenden Aufträgen und Lesestellen.
 * Die Originaldateien auf dem Rechner berührt keiner der Wege.
 */

/** Wie lange der Hinweis mit „Rückgängig“ steht. */
export const UNDO_MS = 12_000;

export type RemoveScope = "trash" | "delete";

export async function trashBooks(books: LibraryBook[]): Promise<void> {
  const r = await commands.libraryTrash(books.map((b) => b.id));
  if (r.status !== "ok") throw new Error(r.error);
}

export async function restoreBooks(books: LibraryBook[]): Promise<void> {
  const r = await commands.libraryRestore(books.map((b) => b.id));
  if (r.status !== "ok") throw new Error(r.error);
}

export async function listTrash(): Promise<TrashEntry[]> {
  const r = await commands.libraryTrashList();
  return r.status === "ok" ? r.data : [];
}

/** Belegter Platz der Bücher in Bytes (unbekannt: 0). */
export async function sizesOf(books: LibraryBook[]): Promise<number> {
  if (!books.length) return 0;
  const r = await commands.librarySizes(books.map((b) => b.id));
  return r.status === "ok" ? r.data.reduce((a, b) => a + b, 0) : 0;
}

/**
 * Endgültig löschen, was schon im Papierkorb liegt: Ordner, Aufträge,
 * Hörstelle sowie Text bzw. Transkript im Vorlesen samt Lesestelle.
 */
export async function purgeBooks(books: LibraryBook[]): Promise<void> {
  const r = await commands.libraryPurge(books.map((b) => b.id));
  // Bleibt das Buch im Papierkorb, bleibt auch sein Text.
  if (r.status !== "ok") throw new Error(r.error);
  for (const book of books) {
    for (const job of pendingJobs(book)) await commands.meetingsDelete(job);
    removeProgress(audioProgressKey(book.id));
    if (book.text) {
      removeProgress(textProgressKey(book.text.page_id));
      await commands.pagesDelete(book.text.page_id);
    }
  }
  await flushProgress();
}

/** Sofort endgültig löschen (aus der Bibliothek heraus). */
export async function deleteBooks(books: LibraryBook[]): Promise<void> {
  await trashBooks(books);
  await purgeBooks(books);
}

/** Was länger als 30 Tage im Papierkorb liegt, wegräumen. */
export async function purgeStaleTrash(): Promise<void> {
  await commands.libraryPurge(null);
}

export function formatBytes(bytes: number, locale = "de"): string {
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${Math.max(1, Math.round(bytes / 1024)).toLocaleString(locale)} KB`;
  if (mb >= 1024)
    return `${(mb / 1024).toLocaleString(locale, { maximumFractionDigits: 1 })} GB`;
  return `${mb.toLocaleString(locale, { maximumFractionDigits: mb < 10 ? 1 : 0 })} MB`;
}
