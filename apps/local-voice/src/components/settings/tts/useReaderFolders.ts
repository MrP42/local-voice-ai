import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type LibraryBook, type PageInfo } from "@/bindings";
import { usePersistentState } from "../../../hooks/usePersistentState";
import {
  FOLDER_ALL,
  followFolder,
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
  // Für welche Seitenmenge die gelieferten Bücher gelten. Weicht sie von
  // `pages` ab, läuft (oder fehlt) die Anfrage dazu: das ist im selben Render
  // bekannt, nicht erst nach einem Effekt. Solange folgt die Auswahl der
  // offenen Seite nicht, sonst bestimmen veraltete Bücher den Ordner (eine
  // neue Buchseite landete unter „Ohne Buch“).
  const [loadedFor, setLoadedFor] = useState<PageInfo[] | null>(null);
  // Eine Anfrage nach einem Sync-Ereignis (Seitenmenge unverändert).
  const [syncPending, setSyncPending] = useState(false);
  const pending = !active || loadedFor !== pages || syncPending;
  const [stored, setStored] = usePersistentState<string>(
    "tts.folder",
    FOLDER_ALL,
  );

  // Nur die jüngste Anfrage zählt: überlappende Antworten können in
  // umgekehrter Reihenfolge eintreffen und sonst neue Bücher überschreiben.
  const request = useRef(0);
  const pagesRef = useRef(pages);
  pagesRef.current = pages;
  const reloadBooks = useCallback(async (forPages: PageInfo[]) => {
    const mine = ++request.current;
    const result = await commands.libraryList().catch(() => null);
    if (mine !== request.current) return;
    if (result?.status === "ok" && Array.isArray(result.data))
      setBooks(result.data);
    setLoadedFor(forPages);
    setSyncPending(false);
  }, []);
  // Beim Abbau gilt keine ausstehende Antwort mehr.
  useEffect(
    () => () => {
      request.current += 1;
    },
    [],
  );

  // Beim Sichtbarwerden und bei jeder neuen Seitenmenge neu laden: Bücher
  // entstehen und verschwinden in der Bibliothek, während Vorlesen im
  // Hintergrund erhalten bleibt.
  useEffect(() => {
    if (active) void reloadBooks(pages);
  }, [active, pages, reloadBooks]);

  useEffect(() => {
    const unlisten = listen("sync-changed", () => {
      setSyncPending(true);
      void reloadBooks(pagesRef.current);
    });
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

  // Der offenen Seite folgen, wenn sie wechselt (Bibliothek, Suche, Löschen)
  // oder wenn neue Bücherdaten sie aus dem unveränderten Ordner nehmen;
  // nicht, wenn jemand einen Ordner wählt: ein gewählter leerer Ordner bleibt.
  const seen = useRef<{ page: string; selection: string; visible: boolean }>({
    page: "",
    selection: "",
    visible: false,
  });
  useEffect(() => {
    if (pending || !activePage || !allIds.includes(activePage)) return;
    const visible = visibleIds.includes(activePage);
    const last = seen.current;
    seen.current = { page: activePage, selection, visible };
    const pageChanged = last.page !== activePage;
    const movedOut = last.selection === selection && last.visible && !visible;
    if (!pageChanged && !movedOut) return;
    const target = followFolder(folders, visibleIds, allIds, activePage);
    if (target) setStored(target);
  }, [pending, activePage, selection, visibleIds, allIds, folders, setStored]);

  /** Ordner wählen; liegt die offene Seite nicht darin, öffnet die erste. */
  const select = useCallback(
    (key: string, open: (id: string) => void) => {
      setStored(key);
      const ids = pagesInFolder(folders, key, allIds);
      const opens = ids.length > 0 && !ids.includes(activePage);
      if (opens) open(ids[0]);
      // Eine bewusste Wahl gilt als gesehen, auch während Bücher laden:
      // sonst überschriebe die spätere Seitenfolge sie.
      seen.current = {
        page: opens ? ids[0] : activePage,
        selection: key,
        visible: opens || ids.includes(activePage),
      };
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
