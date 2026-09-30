import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type Folder, type Meeting } from "@/bindings";
import { notifyMeetingsChanged, useMeetingsChanged } from "@/lib/meetingsBus";
import { folderErrorText } from "../search/FolderChips";
import {
  NO_PROJECT,
  dropResult,
  movedOrder,
  resolveSelection,
  type DropMode,
} from "./projectModel";
import { setSelectedProject, useSelectedProject } from "./selectedProject";

/**
 * Projekte der Aufnahmen-Seite = Ordner der obersten Ebene des Meeting-Stores
 * (keine zweite Struktur). Dieser Hook haelt die Ordnerliste, die Zaehler fuer
 * "Alle Aufnahmen" / "Ohne Projekt", die gewaehlte Auswahl (ueber Neustarts
 * gemerkt) und alle Aenderungen (anlegen, umbenennen, sortieren, loeschen,
 * Besprechung zuordnen). Fehler des Backends kommen als Text zurueck bzw. als
 * Hinweis, nie als Absturz.
 */
export function useProjects() {
  const { t } = useTranslation();
  const [folders, setFolders] = useState<Folder[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [counts, setCounts] = useState<{
    all: number | null;
    none: number | null;
  }>({ all: null, none: null });
  const { selection: rawSelection } = useSelectedProject();
  const setSelection = setSelectedProject;
  const foldersRef = useRef(folders);
  foldersRef.current = folders;

  const reload = useCallback(async () => {
    const [list, count] = await Promise.all([
      commands.meetingFoldersList(),
      commands.meetingFoldersCounts(),
    ]);
    // Aeltere Backends/Attrappen liefern `null`: dann eben keine Projekte.
    if (list.status === "ok") setFolders(list.data ?? []);
    setCounts(
      count.status === "ok" && count.data
        ? { all: count.data.all, none: count.data.unfiled }
        : { all: null, none: null },
    );
    setLoaded(true);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Loeschen, Verschieben oder Import an anderer Stelle (Detailkopf, Bedien-
  // spalte): Zaehler und Ordner neu laden.
  useMeetingsChanged(() => void reload());

  // Eine geloeschte Auswahl faellt auf "Alle Aufnahmen" zurueck; vor dem
  // ersten Laden gilt der gemerkte Wert unveraendert.
  const selection = loaded
    ? resolveSelection(rawSelection, folders)
    : rawSelection;
  useEffect(() => {
    if (loaded && selection !== rawSelection) setSelection(selection);
  }, [loaded, selection, rawSelection, setSelection]);

  const errorText = useCallback(
    (result: { status: string; error?: string }) =>
      folderErrorText(result.status === "error" ? (result.error ?? "") : "", t),
    [t],
  );

  /** `null` = gespeichert, sonst der Fehlertext fuer die Eingabezeile. */
  const create = useCallback(
    async (name: string): Promise<string | null> => {
      const result = await commands.meetingFoldersSave(null, name, null);
      if (result.status !== "ok" || !result.data) return errorText(result);
      await reload();
      setSelection(result.data.id);
      return null;
    },
    [reload, setSelection, errorText],
  );

  const rename = useCallback(
    async (id: string, name: string): Promise<string | null> => {
      const current = foldersRef.current.find((f) => f.id === id);
      const result = await commands.meetingFoldersSave(
        id,
        name,
        current?.color ?? null,
      );
      if (result.status !== "ok") return errorText(result);
      await reload();
      return null;
    },
    [reload, errorText],
  );

  const remove = useCallback(
    async (id: string): Promise<boolean> => {
      const result = await commands.meetingFoldersDelete(id);
      if (result.status !== "ok") {
        toast.error(errorText(result));
        await reload();
        return false;
      }
      // Die Besprechungen der Liste haben sich geaendert; die Meldung laedt
      // auch die Projekte neu.
      notifyMeetingsChanged();
      return true;
    },
    [reload, errorText],
  );

  const move = useCallback(
    async (id: string, direction: -1 | 1) => {
      const next = movedOrder(
        foldersRef.current.map((f) => f.id),
        id,
        direction,
      );
      if (!next) return;
      // Sofort umsortieren, das Backend bestaetigt mit dem naechsten Laden.
      setFolders((prev) =>
        next
          .map((fid) => prev.find((f) => f.id === fid))
          .filter((f): f is Folder => f !== undefined),
      );
      const result = await commands.meetingFoldersReorder(next);
      if (result.status !== "ok") toast.error(errorText(result));
      await reload();
    },
    [reload, errorText],
  );

  /**
   * Eine Besprechung einem Projekt zuordnen (Ziehen, Strg+Ziehen). Die Toast-
   * Meldung bietet "Rueckgaengig" an: sie stellt die vorherigen Projekte her.
   */
  const assign = useCallback(
    async (
      meeting: Pick<Meeting, "id" | "title">,
      from: string,
      target: string,
      mode: DropMode,
    ) => {
      const current = await commands.meetingsGetFolders(meeting.id);
      if (current.status !== "ok") {
        toast.error(errorText(current));
        return;
      }
      const before = current.data ?? [];
      const after = dropResult(before, from, target, mode);
      if (!after) return;
      const saved = await commands.meetingsSetFolders(meeting.id, after);
      if (saved.status !== "ok") {
        toast.error(errorText(saved));
        await reload();
        return;
      }
      notifyMeetingsChanged();
      const name = foldersRef.current.find((f) => f.id === target)?.name ?? "";
      const text =
        target === NO_PROJECT
          ? t("meetings.projects.droppedNone", { title: meeting.title })
          : mode === "add"
            ? t("meetings.projects.droppedAdded", {
                title: meeting.title,
                project: name,
              })
            : t("meetings.projects.droppedMoved", {
                title: meeting.title,
                project: name,
              });
      toast(text, {
        action: {
          label: t("meetings.projects.undo"),
          onClick: () => {
            const alive = before.filter((id) =>
              foldersRef.current.some((f) => f.id === id),
            );
            void commands
              .meetingsSetFolders(meeting.id, alive)
              .then(() => notifyMeetingsChanged());
          },
        },
      });
    },
    [reload, t, errorText],
  );

  return useMemo(
    () => ({
      folders,
      counts,
      selection,
      select: setSelection,
      reload,
      create,
      rename,
      remove,
      move,
      assign,
    }),
    [
      folders,
      counts,
      selection,
      setSelection,
      reload,
      create,
      rename,
      remove,
      move,
      assign,
    ],
  );
}

export type ProjectsApi = ReturnType<typeof useProjects>;
