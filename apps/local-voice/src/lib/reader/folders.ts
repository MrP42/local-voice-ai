// Buch-Ordner im Vorlesen (lva#365, P1): ordnet die flache Seitenliste in
// Bücher (je Buch alle Seiten seiner Fassungen) und lose Texte. Reine
// Abbildung ohne React und Tauri, damit `node --experimental-strip-types
// --test` sie prüfen kann. Noch kein Datenumbau: die Zuordnung kommt aus
// `buch.json` (Haupttext und Fassungen verweisen per `page_id` auf Seiten).

export interface FolderPage {
  id: string;
}

interface TextRef {
  page_id: string;
}

export interface FolderBook {
  id: string;
  title: string;
  text?: TextRef | null;
  editions?: { lang: string; text?: TextRef | null }[] | null;
}

export interface BookFolder {
  bookId: string;
  title: string;
  /** Seiten des Buchs in der Reihenfolge der Seitenliste. */
  pageIds: string[];
}

export interface PageFolders {
  books: BookFolder[];
  /** Seiten ohne Buch, in der Reihenfolge der Seitenliste. */
  loose: string[];
}

/** Seiten-IDs eines Buchs: Haupttext und alle Fassungen, ohne Doppel. */
export const bookPageIds = (book: FolderBook): string[] => {
  const ids = [
    book.text?.page_id,
    ...(book.editions ?? []).map((e) => e.text?.page_id),
  ];
  return [...new Set(ids.filter((id): id is string => !!id))];
};

/**
 * Teilt die Seiten in Buch-Ordner und lose Texte. Bücher ohne vorhandene Seite
 * (reines Hörbuch, Seite gelöscht) erscheinen nicht. Gehört eine Seite zu
 * mehreren Büchern, gewinnt das erste in der Bibliotheksreihenfolge.
 */
export const groupPages = (
  pages: FolderPage[],
  books: FolderBook[],
): PageFolders => {
  const order = new Map(pages.map((p, i) => [p.id, i]));
  const owner = new Map<string, string>();
  for (const book of books) {
    for (const id of bookPageIds(book)) {
      if (order.has(id) && !owner.has(id)) owner.set(id, book.id);
    }
  }
  const folders: BookFolder[] = [];
  for (const book of books) {
    const pageIds = bookPageIds(book)
      .filter((id) => owner.get(id) === book.id)
      .sort((a, b) => (order.get(a) ?? 0) - (order.get(b) ?? 0));
    if (pageIds.length > 0) {
      folders.push({ bookId: book.id, title: book.title, pageIds });
    }
  }
  const loose = pages.map((p) => p.id).filter((id) => !owner.has(id));
  return { books: folders, loose };
};

/** Buch-Ordner, in dem die Seite liegt (oder `null` für einen losen Text). */
export const folderOf = (folders: PageFolders, pageId: string): string | null =>
  folders.books.find((b) => b.pageIds.includes(pageId))?.bookId ?? null;

/** Auswahl in der Ordnerleiste: alle Seiten, lose Texte oder ein Buch. */
export const FOLDER_ALL = "all";
export const FOLDER_TEXTS = "texts";
export const bookFolderKey = (bookId: string) => `book:${bookId}`;

/** Ist die gespeicherte Auswahl noch gültig? Sonst „Alle“. */
export const resolveFolder = (folders: PageFolders, key: string): string =>
  key === FOLDER_TEXTS ||
  folders.books.some((b) => bookFolderKey(b.bookId) === key)
    ? key
    : FOLDER_ALL;

/** Seiten-IDs, die die Liste für die Auswahl zeigt (Reihenfolge der Seitenliste). */
export const pagesInFolder = (
  folders: PageFolders,
  key: string,
  allIds: string[],
): string[] => {
  if (key === FOLDER_TEXTS) return folders.loose;
  const book = folders.books.find((b) => bookFolderKey(b.bookId) === key);
  return book ? book.pageIds : allIds;
};

/** Sprache je Seite aus den Fassungen der Bücher („de“, „en“ …). */
export const pageLanguages = (books: FolderBook[]): Map<string, string> => {
  const out = new Map<string, string>();
  for (const book of books) {
    for (const e of book.editions ?? []) {
      const id = e.text?.page_id;
      if (id && !out.has(id)) out.set(id, e.lang);
    }
  }
  return out;
};

/**
 * Ordner, in den die Auswahl der offenen Seite folgen muss, oder `null`.
 * Nur auf der Grundlage abgeschlossener Bücherdaten aufrufen: mit veralteten
 * Büchern landet eine neue Buchseite sonst unter „Ohne Buch“.
 */
export function followFolder(
  folders: PageFolders,
  visibleIds: string[],
  allIds: string[],
  activePage: string,
): string | null {
  if (!activePage || !allIds.includes(activePage)) return null;
  if (visibleIds.includes(activePage)) return null;
  const owner = folderOf(folders, activePage);
  return owner ? bookFolderKey(owner) : FOLDER_TEXTS;
}
