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
  // Bleibt das Buch im Papierkorb, bleibt auch sein Text.
  if (r.status !== "ok") throw new Error(r.error);
  forgetPending(books);
  for (const book of books) {
    for (const job of pendingJobs(book)) await commands.meetingsDelete(job);
    removeProgress(audioProgressKey(book.id));
    if (scope === "everything" && book.text) {
      removeProgress(textProgressKey(book.text.page_id));
      await commands.pagesDelete(book.text.page_id);
    }
  }
  await flushProgress();
}

/**
 * Offene Entfernungen merken: endet die App im Rückgängig-Fenster, holt der
 * nächste Start das endgültige Löschen nach (samt Text bei „alles“).
 */
const PENDING_KEY = "lva.ui.library.pendingRemovals";
interface Pending {
  books: LibraryBook[];
  scope: RemoveScope;
  /** Zeitpunkt des Entfernens (ms). */
  at: number;
}
function readPending(): Pending[] {
  try {
    const raw = window.localStorage.getItem(PENDING_KEY);
    const list: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? (list as Pending[]) : [];
  } catch {
    return [];
  }
}
function writePending(list: Pending[]) {
  try {
    if (list.length)
      window.localStorage.setItem(PENDING_KEY, JSON.stringify(list));
    else window.localStorage.removeItem(PENDING_KEY);
  } catch {
    /* ohne Speicher räumt der Kern den Papierkorb nach sieben Tagen */
  }
}
export function rememberPending(books: LibraryBook[], scope: RemoveScope) {
  writePending([...readPending(), { books, scope, at: Date.now() }]);
}
export function forgetPending(books: LibraryBook[]) {
  const ids = new Set(books.map((b) => b.id));
  writePending(
    readPending().filter((p) => !p.books.some((b) => ids.has(b.id))),
  );
}

/**
 * Beim Öffnen: liegengebliebene Entfernungen abschließen, dann Altlasten im
 * Papierkorb (älter als sieben Tage) wegräumen.
 */
export async function purgeStaleTrash(): Promise<void> {
  // Nur, was außerhalb des Rückgängig-Fensters liegt: ein Bereichswechsel
  // hängt die Seite neu ein, der Hinweis mit „Rückgängig“ steht dann noch.
  const due = readPending().filter((p) => Date.now() - (p.at ?? 0) > UNDO_MS + 2000);
  for (const p of due) {
    try {
      await finalizeRemoval(p.books, p.scope);
    } catch {
      forgetPending(p.books);
    }
  }
  await commands.libraryPurge(null);
}
