import { commands, type LibraryBook } from "@/bindings";
import { pendingJobs } from "./books";
import {
  audioProgressKey,
  flushProgress,
  removeProgress,
  textProgressKey,
} from "./progress";

/**
 * Einträge entfernen mit Rückgängig: erst Papierkorb (der Kern verschiebt den
 * Buchordner), endgültig erst, wenn das Rückgängig-Fenster zu ist. Die
 * Originaldateien auf dem Rechner berührt das nie, nur die Kopien der
 * Bibliothek.
 */

/** Wie lange „Rückgängig“ angeboten wird. */
export const UNDO_MS = 10_000;

export type RemoveScope = "library" | "everything";

export async function trashBooks(books: LibraryBook[]): Promise<void> {
  const r = await commands.libraryTrash(books.map((b) => b.id));
  if (r.status !== "ok") throw new Error(r.error);
}

export async function restoreBooks(books: LibraryBook[]): Promise<void> {
  const r = await commands.libraryRestore(books.map((b) => b.id));
  if (r.status !== "ok") throw new Error(r.error);
}

/**
 * Endgültig: Papierkorb leeren, Transkriptions-Aufträge und Hörstelle weg;
 * bei `everything` auch der Text im Vorlesen samt Lesestelle.
 */
export async function finalizeRemoval(
  books: LibraryBook[],
  scope: RemoveScope,
): Promise<void> {
  const r = await commands.libraryPurge(books.map((b) => b.id));
  for (const book of books) {
    for (const job of pendingJobs(book)) await commands.meetingsDelete(job);
    removeProgress(audioProgressKey(book.id));
    if (scope === "everything" && book.text) {
      removeProgress(textProgressKey(book.text.page_id));
      await commands.pagesDelete(book.text.page_id);
    }
  }
  await flushProgress();
  if (r.status !== "ok") throw new Error(r.error);
}

/** Altlasten im Papierkorb (Absturz im Rückgängig-Fenster) wegräumen. */
export async function purgeStaleTrash(): Promise<void> {
  await commands.libraryPurge(null);
}
