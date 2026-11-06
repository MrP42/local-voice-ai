/**
 * Aktionen auf Sprachfassungen (lva#344): dünne Hülle um die Kernbefehle aus
 * PR 1. Jede wirft bei einem Fehler eine `Error` mit der Meldung des Kerns.
 */
import {
  commands,
  type Alignment,
  type LanguagePair,
  type LibraryBook,
} from "@/bindings";
import {
  audioKeyOf,
  deviceId,
  editionsOf,
  langName,
  textKeyOf,
} from "./editions";
import { loadProgress } from "./progress";

type Answer<T> = { status: "ok"; data: T } | { status: "error"; error: string };

function unwrap<T>(r: Answer<T>): T {
  if (r.status !== "ok") throw new Error(r.error);
  return r.data;
}

/** Bücher als Sprachfassungen in `target` übernehmen. */
export async function joinEditions(
  targetId: string,
  sourceIds: string[],
): Promise<LibraryBook> {
  return unwrap(await commands.libraryJoinEditions(targetId, sourceIds));
}

/** Sprachfassung als eigenes Buch abtrennen: [verbleibend, neu]. */
export async function splitEdition(
  bookId: string,
  editionId: string,
  newPrimary: string | null,
): Promise<LibraryBook[]> {
  return unwrap(
    await commands.librarySplitEdition(bookId, editionId, newPrimary),
  );
}

/** Sprachfassung löschen (Papierkorb, 30 Tage). */
export async function removeEdition(
  bookId: string,
  editionId: string,
  newPrimary: string | null,
): Promise<LibraryBook> {
  return unwrap(
    await commands.libraryRemoveEdition(bookId, editionId, newPrimary),
  );
}

/** Gelöschte Sprachfassung zurückholen. */
export async function restoreEdition(
  bookId: string,
  editionId: string,
): Promise<LibraryBook> {
  return unwrap(
    await commands.libraryRestoreEdition(bookId, editionId, deviceId()),
  );
}

export async function setPrimaryEdition(
  bookId: string,
  editionId: string,
): Promise<LibraryBook> {
  return unwrap(
    await commands.librarySetPrimaryEdition(bookId, editionId, deviceId()),
  );
}

export async function saveAlignment(
  bookId: string,
  alignment: Alignment,
): Promise<LibraryBook> {
  return unwrap(await commands.librarySetAlignment(bookId, alignment));
}

export async function languagePairs(): Promise<LanguagePair[]> {
  try {
    const r = await commands.libraryLanguagePairs();
    return r.status === "ok" && Array.isArray(r.data) ? r.data : [];
  } catch {
    return [];
  }
}

/** Zuletzt gehört oder gelesen (Unix-ms) über alle Fassungen, 0 = nie. */
export function lastUsed(book: LibraryBook): number {
  return editionsOf(book).reduce((latest, e) => {
    const keys = [textKeyOf(e), e.audio ? audioKeyOf(book, e) : null];
    const updated = keys.map((k) => (k ? (loadProgress(k)?.updated ?? 0) : 0));
    return Math.max(latest, ...updated);
  }, 0);
}

/**
 * Welches Buch eines Paars die Hauptfassung stellt: das zuletzt benutzte
 * (Titel und Cover wie in sf-02), bei Gleichstand das ältere.
 */
export function pairTarget(
  a: LibraryBook,
  b: LibraryBook,
): { target: LibraryBook; source: LibraryBook } {
  const [ua, ub] = [lastUsed(a), lastUsed(b)];
  const aFirst = ua !== ub ? ua > ub : (a.added_ms ?? 0) <= (b.added_ms ?? 0);
  return aFirst ? { target: a, source: b } : { target: b, source: a };
}

/** „Deutsch, Hörbuch“ – Sprachen und Formate eines Buchs für die Rückfrage. */
export function describeBook(
  book: LibraryBook,
  kinds: { audio: string; text: string },
): string {
  const list = editionsOf(book);
  const langs = [...new Set(list.map((e) => langName(e.lang)).filter(Boolean))];
  const has = [
    list.some((e) => e.audio) ? kinds.audio : null,
    list.some((e) => e.text) ? kinds.text : null,
  ].filter(Boolean);
  return [...langs, ...has].join(", ");
}
