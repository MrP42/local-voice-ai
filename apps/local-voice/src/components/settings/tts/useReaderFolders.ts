import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type LibraryBook, type PageInfo } from "@/bindings";
import { usePersistentState } from "../../../hooks/usePersistentState";
import {
  FOLDER_ALL,
  FOLDER_TEXTS,
  bookFolderKey,
  folderOf,
  groupPages,
  pageLanguages,
  pagesInFolder,
  resolveFolder,
} from "@/lib/reader/folders";

/**
 * Ordnerleiste im Vorlesen (lva#365 P1): lädt die Bücher, ordnet die Seiten
 * in Buch-Ordner und lose Texte und hält die Auswahl. Öffnet jemand eine
 * Seite außerhalb der Auswahl (Bibliothek, Suche), wechselt die Auswahl in
 * ihren Ordner, damit die offene Seite in der Liste sichtbar bleibt.
 */
export function useReaderFolders(
  pages: PageInfo[],
  activePage: string,
  active: boolean,
) {
  const [books, setBooks] = useState<LibraryBook[]>([]);
  // Erst nach dem ersten Laden der Bücher folgt die Auswahl der offenen
  // Seite; sonst springt sie beim Start über „Alle“ in den falschen Ordner.
  const [booksLoaded, setBooksLoaded] = useState(false);
  const [stored, setStored] = usePersistentState<string>(
    "tts.folder",
    FOLDER_ALL,
  );

  const reloadBooks = useCallback(async () => {
    const result = await commands.libraryList().catch(() => null);
    if (result?.status === "ok" && Array.isArray(result.data))
      setBooks(result.data);
    setBooksLoaded(true);
  }, []);

  // Beim Sichtbarwerden neu laden: Bücher entstehen und verschwinden in der
  // Bibliothek, während Vorlesen im Hintergrund erhalten bleibt.
  useEffect(() => {
    if (active) void reloadBooks();
  }, [active, pages, reloadBooks]);

  useEffect(() => {
    const unlisten = listen("sync-changed", () => void reloadBooks());
    return () => {
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [reloadBooks]);

  const folders = useMemo(() => groupPages(pages, books), [pages, books]);
  const selection = resolveFolder(folders, stored);
  const allIds = useMemo(() => pages.map((p) => p.id), [pages]);
  const visibleIds = useMemo(
    () => pagesInFolder(folders, selection, allIds),
    [folders, selection, allIds],
  );
  const langOf = useMemo(() => pageLanguages(books), [books]);

  useEffect(() => {
    if (!booksLoaded) return;
    if (!activePage || !allIds.includes(activePage)) {
      // Keine gültige offene Seite: eine leere Auswahl auf „Alle“ zurück.
      if (visibleIds.length === 0 && allIds.length > 0) setStored(FOLDER_ALL);
      return;
    }
    if (visibleIds.includes(activePage)) return;
    const owner = folderOf(folders, activePage);
    setStored(owner ? bookFolderKey(owner) : FOLDER_TEXTS);
  }, [booksLoaded, activePage, visibleIds, allIds, folders, setStored]);

  /** Ordner wählen; liegt die offene Seite nicht darin, öffnet die erste. */
  const select = useCallback(
    (key: string, open: (id: string) => void) => {
      setStored(key);
      const ids = pagesInFolder(folders, key, allIds);
      if (ids.length > 0 && !ids.includes(activePage)) open(ids[0]);
    },
    [folders, allIds, activePage, setStored],
  );

  return {
    books,
    folders,
    selection,
    select,
    visibleIds,
    langOf,
    total: pages.length,
  };
}
