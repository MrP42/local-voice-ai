import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  commands,
  events,
  type Meeting,
  type MeetingSearchItem,
  type ScopeFilter,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Alert } from "../../ui/Alert";
import { IconAction } from "../../ui/IconAction";
import { ActionMenu } from "../../ui/ActionMenu";
import {
  Check,
  CheckSquare,
  FilePlus,
  ListChecks,
  Menu,
  MessageSquare,
  Plus,
  X,
} from "lucide-react";
import { SearchBar, SearchSnippet } from "./search/SearchBar";
import { EMPTY_FILTER, type ListFilter } from "./search/FilterChips";
import { ContextMenu } from "./search/FolderChips";
import { formatMeetingDate } from "@/lib/meetingDate";
import type { PersonRef } from "./people/PersonPopover";
import { FolderPickerDialog } from "./search/FolderPickerDialog";
import { JobBar } from "./JobProgress";
import { useMeetingProgress } from "@/hooks/useMeetingJobs";
import { useImportQueue } from "@/hooks/useImportQueue";
import { heldForRecording, queuePlace } from "@/lib/meetingQueue";
import { QueueChip } from "./QueueStatus";
import { notifyMeetingsChanged, useMeetingsChanged } from "@/lib/meetingsBus";
import type { BriefInfo } from "@/bindings";
import { ProjectRow } from "./projects/ProjectRow";
import { ProjectFilter, activeFilterCount } from "./projects/ProjectFilter";
import { NextUp } from "./projects/NextUp";
import type { ProjectsApi } from "./projects/useProjects";
import type { useMeetingDrag } from "./projects/useMeetingDrag";
import { ALL_PROJECTS, NO_PROJECT } from "./projects/projectModel";
import {
  ProjectMinutesDialog,
  type ProjectMinutesChoice,
} from "./projectMinutes/ProjectMinutesDialog";
import { ProjectMinutesRows } from "./projectMinutes/ProjectMinutesRows";
import type { OpenProjectMinutes } from "./projectMinutes/ProjectMinutesView";
import {
  useProjectCandidates,
  useProjectMinutesList,
} from "./projectMinutes/useProjectMinutes";
import {
  MIN_RECORDINGS,
  eligibleIds,
  projectMinutesJobKey,
  pruneSelection,
} from "@/lib/projectMinutes";

const PAGE_SIZE = 25;
const DAY_SECONDS = 86_400;

// Windows paths use backslashes; the old class `[\/]` matched only the
// forward slash, so a C:\... path came back whole.
const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const statusChipClass = (status: string, source?: string) => {
  // G1: ein leerer Eintrag ist nicht "fertig", sondern leer.
  if (source === "empty" && status === "ready") {
    return "border border-dashed border-mid-gray/40 text-text/70";
  }
  switch (status) {
    case "ready":
      return "bg-green-500/20 text-green-400";
    case "recording":
      return "bg-logo-primary text-on-accent";
    default:
      return "bg-mid-gray/20 text-text/70";
  }
};

const formatDuration = (durationMs: number | null) => {
  if (durationMs === null) return "--:--";
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

/** Menue an der Zeile statt an der Maus, wenn es die Kontexttaste ausloest. */
const menuPoint = (e: React.MouseEvent<HTMLElement>) => {
  if (e.clientX === 0 && e.clientY === 0) {
    const rect = e.currentTarget.getBoundingClientRect();
    return { x: rect.left + 16, y: rect.bottom };
  }
  return { x: e.clientX, y: e.clientY };
};

type MeetingDragApi = ReturnType<typeof useMeetingDrag>;

interface MeetingListProps {
  projects: ProjectsApi;
  drag: MeetingDragApi;
  /** Platz im Kopf der Projekte-Spalte fuer die Symbolknoepfe (Portal). */
  actionsEl: HTMLElement | null;
  onSelect: (meeting: Meeting) => void;
  /** Die gewaehlte Besprechung: wird hervorgehoben, ihre Aenderungen (Titel,
      Status) landen sofort in der Zeile. */
  selected?: Meeting | null;
  /** Eine Besprechung wurde geloescht (die gewaehlte braucht dann Ersatz). */
  onDeleted?: (id: string) => void;
  /** M4-P4e: Chat ueber viele Besprechungen oeffnen (Scope vorbelegt). */
  onAsk?: (filter: ScopeFilter) => void;
  /** M5-P5e: "Vorbereiten" an einem Termin des Abschnitts "Als Naechstes". */
  onPrepare?: (info: BriefInfo) => void;
  /** Die Besprechung, die gerade aufgenommen wird: sie laesst sich nicht loeschen. */
  liveId?: string | null;
  /** M5-P5d: Filter "Person: Anna Berg" (kommt aus dem Popover der Detailansicht). */
  personFilter?: PersonRef | null;
  onPersonFilterChange?: (person: PersonRef | null) => void;
  /**
   * G1 (#70): "Neue Besprechung" (ein leerer Eintrag). `folderId`: das Projekt,
   * in das er kommt (`null` = ohne Projekt); `view`: die Zeile der Spalte, die
   * danach gewaehlt wird, damit der neue Eintrag zu sehen ist.
   */
  onNewMeeting?: (folderId: string | null, view: string) => void;
  /**
   * G3 (#70, U9): Aufnahmen eines Projekts gemeinsam protokollieren. Die Liste
   * kennt die Auswahl; der Lauf selbst gehoert dem Aufrufer.
   */
  onStartProjectMinutes?: (request: {
    folderId: string;
    meetingIds: string[];
    templateId: string;
    kind: ProjectMinutesChoice["kind"];
  }) => void;
  /** Ein Projekt-Protokoll in der Arbeitsflaeche oeffnen. */
  onOpenProjectMinutes?: (id: string) => void;
  /** Die Ansicht des laufenden Laufs eines Projekts oeffnen. */
  onOpenProjectRun?: (folderId: string) => void;
  /** Was die Arbeitsflaeche gerade als Projekt-Protokoll zeigt. */
  activeProjectMinutes?: OpenProjectMinutes | null;
}

/**
 * Projekte-Spalte der Aufnahmen-Seite (Variante B): "Alle Aufnahmen", die
 * Projekte (= Ordner der obersten Ebene) und "Ohne Projekt"; unter der
 * gewaehlten Zeile stehen ihre Besprechungen. Suche und Filter wirken
 * innerhalb der Auswahl. Besprechungen lassen sich auf ein Projekt ziehen
 * (Strg = hinzufuegen, n:m) oder ueber das Kontextmenue zuordnen.
 */
export const MeetingList: React.FC<MeetingListProps> = ({
  projects,
  drag,
  actionsEl,
  onSelect,
  selected = null,
  onDeleted,
  onAsk,
  onPrepare,
  liveId = null,
  personFilter = null,
  onPersonFilterChange,
  onNewMeeting,
  onStartProjectMinutes,
  onOpenProjectMinutes,
  onOpenProjectRun,
  activeProjectMinutes = null,
}) => {
  const { t, i18n } = useTranslation();
  // P8a: laufende Verarbeitungen (Fortschritt, Restdauer) statt nur "Wird verarbeitet".
  const progressMap = useMeetingProgress();
  // U7: Import-Warteschlange (Platz, warum sie steht).
  const queue = useImportQueue();
  // Ohne Suche/Filter tragen die Eintraege kein Snippet.
  const [items, setItems] = useState<MeetingSearchItem[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [hasMore, setHasMore] = useState(true);
  const [deleteTarget, setDeleteTarget] = useState<Meeting | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadingRef = useRef(false);
  // Jede Ladeanfrage bekommt eine Nummer; eine spaet eintreffende Antwort
  // einer ueberholten Suche darf die aktuelle Liste nicht ueberschreiben.
  const requestRef = useRef(0);

  // M4-P4d: Suche, Filter
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ListFilter>(EMPTY_FILTER);
  const [truncated, setTruncated] = useState(false);
  const [pickerTarget, setPickerTarget] = useState<Meeting | null>(null);
  const [rowMenu, setRowMenu] = useState<{
    x: number;
    y: number;
    meeting: Meeting;
  } | null>(null);
  // M4-P4e: Auswahlmodus fuer "Auswahl fragen".
  const [selecting, setSelecting] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const toggleSelected = (id: string) =>
    setSelectedIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id],
    );

  // G3 (#70, U9): Auswahl fuer ein gemeinsames Protokoll (nur in einem Projekt).
  const [pmMode, setPmMode] = useState(false);
  const [pmIds, setPmIds] = useState<string[]>([]);
  const [pmDialog, setPmDialog] = useState(false);
  const togglePm = (id: string) =>
    setPmIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id],
    );

  // Projekte: Auswahl, Aufklappen, Anlegen, Umbenennen, Menue, Loeschen.
  const { folders, counts, selection } = projects;
  const [listOpen, setListOpen] = useState(true);
  const [creating, setCreating] = useState(false);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [projectMenu, setProjectMenu] = useState<{
    x: number;
    y: number;
    id: string;
  } | null>(null);
  const [deleteProject, setDeleteProject] = useState<string | null>(null);
  // G1: Menue der Zeilen "Alle Aufnahmen" und "Ohne Projekt" (nur "Neue Besprechung hier").
  const [plainMenu, setPlainMenu] = useState<{
    x: number;
    y: number;
    id: typeof ALL_PROJECTS | typeof NO_PROJECT;
  } | null>(null);

  // Wechselt die Auswahl (auch von aussen: Projekt geloescht, Leiste), steht
  // die Liste der neuen Zeile offen.
  useEffect(() => {
    setListOpen(true);
  }, [selection]);

  const folderId =
    selection === ALL_PROJECTS || selection === NO_PROJECT ? null : selection;
  const unfiled = selection === NO_PROJECT;
  const selectionName =
    selection === ALL_PROJECTS
      ? t("meetings.projects.all")
      : selection === NO_PROJECT
        ? t("meetings.projects.none")
        : (folders.find((f) => f.id === selection)?.name ?? "");

  // G3: die Projekt-Protokolle des gewaehlten Projekts, ihr Lauf und die
  // Auskunft, welche Aufnahmen sich waehlen lassen.
  const pmList = useProjectMinutesList(folderId);
  const pmCandidates = useProjectCandidates(folderId, pmMode, items);
  const pmCandidateById = useMemo(
    () =>
      new Map((pmCandidates.candidates ?? []).map((c) => [c.meeting_id, c])),
    [pmCandidates.candidates],
  );
  const pmRunProgress = folderId
    ? progressMap[projectMinutesJobKey(folderId)]
    : undefined;
  // Wechselt das Projekt, endet die Auswahl (sie galt dem alten); wird eine
  // Aufnahme unwaehlbar (etwa weil sie geloescht wurde), faellt sie heraus.
  const pendingPmRef = useRef<string | null>(null);
  useEffect(() => {
    setPmIds([]);
    setPmDialog(false);
    // Ueber das Kontextmenue eines Projekts: erst waehlen, dann beginnt dort die Auswahl.
    if (folderId !== null && pendingPmRef.current === folderId) {
      pendingPmRef.current = null;
      setPmMode(true);
    } else {
      setPmMode(false);
    }
  }, [folderId]);
  useEffect(() => {
    if (pmCandidates.candidates) {
      setPmIds((prev) => {
        const next = pruneSelection(prev, pmCandidates.candidates ?? []);
        return next.length === prev.length ? prev : next;
      });
    }
  }, [pmCandidates.candidates]);

  const personId = personFilter?.id ?? null;
  const filtered =
    query !== "" ||
    folderId !== null ||
    unfiled ||
    personId !== null ||
    filter.rangeDays !== null ||
    filter.source !== null ||
    filter.hasNotes;
  const fromTs = useMemo(
    () =>
      filter.rangeDays === null
        ? null
        : Math.floor(Date.now() / 1000) - filter.rangeDays * DAY_SECONDS,
    [filter.rangeDays],
  );

  const loadPage = useCallback(
    async (offset: number) => {
      const isFirstPage = offset === 0;
      if (!isFirstPage && loadingRef.current) return;
      const request = ++requestRef.current;
      loadingRef.current = true;
      if (isFirstPage) setLoading(true);

      try {
        let pageItems: MeetingSearchItem[];
        let more: boolean;
        let cut = false;
        let count: number | null = null;
        if (!filtered) {
          // Ohne Suchtext und Filter: die bisherige Liste (25er-Seiten).
          const result = await commands.meetingsList(offset, PAGE_SIZE);
          if (request !== requestRef.current) return;
          if (result.status !== "ok") {
            // A failing list used to render as "no meetings yet" — visually
            // indistinguishable from data loss. Say what actually happened.
            setListError(
              t("meetings.errors.listFailed", { error: result.error }),
            );
            setHasMore(false);
            return;
          }
          const data = result.data ?? [];
          pageItems = data.map((meeting) => ({
            meeting,
            snippet: null,
            hit_source: null,
          }));
          more = data.length === PAGE_SIZE;
        } else {
          const result = await commands.meetingsSearch(
            query,
            {
              folder_id: folderId,
              from: fromTs,
              to: null,
              source: filter.source,
              has_notes: filter.hasNotes ? true : null,
              person_id: personId,
              unfiled: unfiled ? true : null,
            },
            offset,
            PAGE_SIZE,
          );
          if (request !== requestRef.current) return;
          if (result.status !== "ok") {
            const reason =
              result.error === "filter_invalid"
                ? t("meetings.search.filterInvalid")
                : result.error;
            setListError(t("meetings.search.failed", { error: reason }));
            setHasMore(false);
            return;
          }
          pageItems = result.data?.items ?? [];
          count = result.data?.total ?? null;
          more =
            pageItems.length > 0 &&
            offset + pageItems.length < (result.data?.total ?? 0);
          cut = result.data?.truncated ?? false;
        }
        setListError(null);
        setItems((prev) => (isFirstPage ? pageItems : [...prev, ...pageItems]));
        setHasMore(more);
        setTruncated(cut);
        setTotal(count);
      } finally {
        if (request === requestRef.current) {
          setLoading(false);
          loadingRef.current = false;
        }
      }
    },
    [
      filtered,
      query,
      folderId,
      unfiled,
      personId,
      fromTs,
      filter.source,
      filter.hasNotes,
      t,
    ],
  );

  useEffect(() => {
    void loadPage(0);
  }, [loadPage]);

  // Die Liste hat sich geaendert (Zuordnung, Loeschen, Import an anderer
  // Stelle): die Besprechung kann aus der gewaehlten Liste gefallen oder
  // hinzugekommen sein. Die Projekte laden sich selbst neu.
  const loadPageRef = useRef(loadPage);
  loadPageRef.current = loadPage;
  useMeetingsChanged(() => void loadPageRef.current(0));

  // Refresh from the current recording/import/generation lifecycle. The
  // import path in particular emits no state events at all — its command
  // result is the only signal — so this only covers the recording/generate
  // paths; import success triggers its own explicit reload below.
  useEffect(() => {
    const un = events.meetingEvent.listen((e) => {
      if (e.payload.kind === "state") {
        void loadPage(0);
        void projects.reload();
      }
    });
    return () => {
      un.then((f) => f());
    };
    // `projects.reload` ist stabil; der Hook liefert ein neues Objekt je Aenderung.
  }, [loadPage, projects.reload]);

  useEffect(() => {
    if (loading) return;
    const sentinel = sentinelRef.current;
    if (!sentinel || !hasMore) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting) {
          void loadPage(items.length);
        }
      },
      { threshold: 0 },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loading, hasMore, loadPage, items.length]);

  // Titel und Status der gewaehlten Besprechung kommen aus der Detailansicht
  // (Umbenennen, Verarbeitung fertig): die Zeile zieht sofort nach.
  useEffect(() => {
    if (!selected) return;
    setItems((prev) =>
      prev.some((item) => item.meeting === selected) ||
      !prev.some((item) => item.meeting.id === selected.id)
        ? prev
        : prev.map((item) =>
            item.meeting.id === selected.id
              ? { ...item, meeting: selected }
              : item,
          ),
    );
  }, [selected]);

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    const id = deleteTarget.id;
    setDeleteTarget(null);
    onDeleted?.(id);
    setDeleteError(null);
    setItems((prev) => prev.filter((item) => item.meeting.id !== id));
    const result = await commands.meetingsDelete(id);
    if (result.status === "error") {
      setDeleteError(t("meetings.errors.deleteFailed"));
      void loadPage(0);
    }
    // Die Zaehler zaehlen nur lebende Besprechungen.
    notifyMeetingsChanged();
  };

  // Projekt und Zeitraum der Liste gelten auch fuer den Chat; Quelle und
  // "mit Notizen" kennt der Chat-Scope nicht. "Ohne Projekt" kennt er auch
  // nicht: dort gehen die IDs der Besprechungen mit.
  const askAll = async () => {
    let meetingIds: string[] | null = null;
    if (unfiled) {
      const result = await commands.meetingsSearch(
        "",
        {
          folder_id: null,
          from: fromTs,
          to: null,
          source: null,
          has_notes: null,
          person_id: personId,
          unfiled: true,
        },
        0,
        100,
      );
      meetingIds =
        result.status === "ok" && result.data
          ? result.data.items.map((item) => item.meeting.id)
          : [];
    }
    onAsk?.({
      meeting_ids: meetingIds,
      folder_id: folderId,
      person: null,
      person_id: personId,
      from: fromTs,
      to: null,
      event_uid: null,
    });
  };

  const askSelection = () => {
    // In Listenreihenfolge, nicht in Klickreihenfolge.
    const order = items.map((item) => item.meeting.id);
    const ids = order.filter((id) => selectedIds.includes(id));
    if (ids.length === 0) return;
    onAsk?.({
      meeting_ids: ids,
      folder_id: null,
      person: null,
      person_id: null,
      from: null,
      to: null,
      event_uid: null,
    });
  };

  const activate = (id: string) => {
    if (selection === id) {
      setListOpen((o) => !o);
      return;
    }
    projects.select(id);
    setListOpen(true);
  };

  const searching = query !== "";
  const dropId = drag.drag?.target ?? null;
  const dragging = drag.drag !== null;

  const meetingRow = (
    { meeting, snippet, hit_source }: MeetingSearchItem,
    nested: boolean,
  ) => {
    const timestamp = meeting.started_at ?? meeting.created_at;
    const date = new Date(timestamp * 1000);
    const dateLabel = formatMeetingDate(date, i18n.language, "compact");
    const isSelected = selected?.id === meeting.id;
    const progress = progressMap[meeting.id];
    const place =
      meeting.status === "queued" ? queuePlace(queue, meeting.id) : null;
    const held = heldForRecording(queue, meeting.id);
    const label = t("meetings.chat.list.selectRow", { title: meeting.title });
    // G3: im Auswahlmodus "Gemeinsam protokollieren" sind nur Aufnahmen mit
    // Transkript waehlbar; die anderen stehen ausgegraut mit ihrem Grund da.
    const checkable = selecting || pmMode;
    const candidate = pmMode ? pmCandidateById.get(meeting.id) : undefined;
    const pickable = !pmMode || candidate?.eligible === true;
    const reason =
      pmMode && candidate && !candidate.eligible ? candidate.reason : null;
    const checked = pmMode
      ? pmIds.includes(meeting.id)
      : selectedIds.includes(meeting.id);
    const toggle = () => {
      if (pmMode) {
        if (pickable) togglePm(meeting.id);
      } else toggleSelected(meeting.id);
    };
    return (
      <div
        key={meeting.id}
        role={checkable ? "checkbox" : "button"}
        aria-checked={checkable ? checked : undefined}
        aria-disabled={pmMode && !pickable ? true : undefined}
        aria-label={checkable ? label : undefined}
        tabIndex={0}
        data-meeting-id={meeting.id}
        data-testid="meeting-row"
        data-pickable={pmMode ? String(pickable) : undefined}
        title={
          reason
            ? t(`meetings.projectMinutes.reasonHint.${reason}`, {
                defaultValue: reason,
              })
            : undefined
        }
        aria-current={isSelected ? "true" : undefined}
        className={`my-px select-none rounded-lg border px-2 py-1.5 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60 ${
          pmMode && !pickable
            ? "cursor-not-allowed opacity-50"
            : "cursor-pointer"
        } ${nested ? "ms-5" : ""} ${
          dragging && drag.drag?.meeting.id === meeting.id
            ? "border-logo-primary/60 opacity-50"
            : isSelected
              ? "border-transparent bg-logo-primary/15"
              : "border-transparent hover:bg-mid-gray/10"
        }`}
        onPointerDown={(e) => {
          if (!checkable) drag.start(e, meeting);
        }}
        onClick={() => {
          if (drag.consumeClick()) return;
          if (checkable) toggle();
          else onSelect(meeting);
        }}
        onKeyDown={(e) => {
          if (e.target !== e.currentTarget) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            if (checkable) toggle();
            else onSelect(meeting);
          }
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          setRowMenu({ ...menuPoint(e), meeting });
        }}
      >
        <div className="flex items-center gap-1.5">
          {checkable && (
            <span
              aria-hidden="true"
              className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border ${
                checked
                  ? "border-logo-primary bg-logo-primary text-on-accent"
                  : "border-mid-gray/60"
              }`}
            >
              {checked && <Check width={12} height={12} />}
            </span>
          )}
          <p className="min-w-0 flex-1 truncate text-sm font-medium">
            {meeting.title}
          </p>
        </div>
        {/* The file an import came from, kept visible even after the title
            was renamed away from it. */}
        {meeting.source_path && (
          <p
            className="truncate text-xs text-text/50"
            title={meeting.source_path}
          >
            {baseName(meeting.source_path)}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-text/60">
          <span>
            {dateLabel} · {formatDuration(meeting.duration_ms)}
            {hit_source && (
              <>
                {" · "}
                {t("meetings.search.hitIn", {
                  source: t(`meetings.search.hitSource.${hit_source}`),
                })}
              </>
            )}
          </span>
          {reason && (
            <span
              className="inline-flex items-center rounded-full border border-mid-gray/40 px-2 text-[11px] font-medium text-text/70"
              data-testid="row-reason"
              data-reason={reason}
            >
              {t(`meetings.projectMinutes.reason.${reason}`, {
                defaultValue: reason,
              })}
            </span>
          )}
          {place ? (
            <QueueChip place={place} />
          ) : (
            !reason &&
            !(
              progress &&
              (meeting.status === "processing" ||
                meeting.status === "recording")
            ) && (
              <span
                className={`inline-flex items-center rounded-full px-2 text-[11px] font-medium ${statusChipClass(meeting.status, meeting.source)}`}
                data-testid="row-status"
              >
                {meeting.source === "empty" && meeting.status === "ready"
                  ? t("meetings.empty.chip")
                  : t(`meetings.status.${meeting.status}`, {
                      defaultValue: meeting.status,
                    })}
              </span>
            )
          )}
        </div>
        {progress &&
          (meeting.status === "processing" ||
            meeting.status === "recording") && (
            <JobBar
              progress={progress}
              heldForRecording={held}
              className="mt-1 w-full"
            />
          )}
        {snippet && <SearchSnippet snippet={snippet} />}
      </div>
    );
  };

  const meetingsBlock = (nested: boolean) => {
    if (loading) {
      return (
        <p className="px-2 py-2 text-sm text-text/60">
          {t("meetings.list.loading")}
        </p>
      );
    }
    if (items.length === 0) {
      return (
        <p className="px-2 py-2 text-sm text-text/60">
          {searching
            ? t("meetings.search.noResults")
            : filtered && (folderId === null || activeFilterCount(filter) > 0)
              ? t("meetings.search.noFilterResults")
              : folderId !== null
                ? t("meetings.projects.emptyProject")
                : t("meetings.list.empty")}
        </p>
      );
    }
    return (
      <div role="list" aria-label={selectionName}>
        {truncated && (
          <p className="px-2 py-1 text-xs text-text/60">
            {t("meetings.search.truncated")}
          </p>
        )}
        {items.map((item) => (
          <div role="listitem" key={item.meeting.id}>
            {meetingRow(item, nested)}
          </div>
        ))}
      </div>
    );
  };

  const projectMenuTarget = projectMenu
    ? folders.find((f) => f.id === projectMenu.id)
    : undefined;
  const projectMenuIndex = projectMenuTarget
    ? folders.indexOf(projectMenuTarget)
    : -1;
  const deleteProjectTarget = deleteProject
    ? folders.find((f) => f.id === deleteProject)
    : undefined;

  const actions = (
    <>
      {onNewMeeting && (
        <IconAction
          size="sm"
          icon={FilePlus}
          label={t("meetings.projects.newMeeting")}
          description={t("meetings.projects.newMeetingHint")}
          testId="projects-new-meeting"
          onClick={() => onNewMeeting(folderId, selection)}
        />
      )}
      <IconAction
        size="sm"
        icon={Plus}
        label={t("meetings.projects.new")}
        description={t("meetings.projects.newHint")}
        testId="projects-add"
        onClick={() => {
          setRenamingId(null);
          setCreating(true);
        }}
      />
      {onAsk && (
        <IconAction
          size="sm"
          icon={MessageSquare}
          label={t("meetings.chat.list.askAll")}
          description={t("meetings.projects.askHint")}
          testId="projects-ask"
          onClick={() => void askAll()}
        />
      )}
      <ActionMenu
        trigger={{
          icon: Menu,
          label: t("meetings.projects.more"),
          description: t("meetings.projects.moreHint"),
          testId: "projects-more",
          size: "sm",
        }}
        menuLabel={t("meetings.projects.more")}
        align="end"
        widthClass="w-56"
        items={[
          ...(onAsk
            ? [
                {
                  id: "select",
                  label: selecting
                    ? t("meetings.chat.list.selectDone")
                    : t("meetings.chat.list.select"),
                  icon: CheckSquare,
                  testId: "projects-select",
                  onSelect: () => {
                    setPmMode(false);
                    setSelecting((on) => !on);
                    setSelectedIds([]);
                  },
                },
              ]
            : []),
          ...(onStartProjectMinutes
            ? [
                {
                  id: "project-minutes",
                  label: t("meetings.projectMinutes.menu"),
                  icon: ListChecks,
                  testId: "projects-pm",
                  disabled: folderId === null,
                  title:
                    folderId === null
                      ? t("meetings.projectMinutes.needProject")
                      : t("meetings.projectMinutes.menuHint"),
                  onSelect: () => {
                    setSelecting(false);
                    setPmIds([]);
                    setPmMode((on) => !on);
                  },
                },
              ]
            : []),
        ]}
      />
    </>
  );

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 rounded-md">
      {actionsEl && createPortal(actions, actionsEl)}

      <div className="flex shrink-0 items-center gap-1.5">
        <SearchBar
          onSearch={setQuery}
          placeholder={t("meetings.projects.searchIn", {
            name: selectionName,
          })}
        />
        <ProjectFilter value={filter} onChange={setFilter} />
      </div>

      {pmMode && (
        <div
          role="group"
          aria-label={t("meetings.projectMinutes.bar.label")}
          data-testid="pm-bar"
          className="shrink-0 space-y-1.5 rounded-md bg-logo-primary/10 px-3 py-1.5"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm" data-testid="pm-count">
              {t("meetings.projectMinutes.bar.selected", {
                count: pmIds.length,
              })}
            </span>
            <span className="flex items-center gap-1">
              <Button
                size="sm"
                variant="ghost"
                data-testid="pm-all"
                title={t("meetings.projectMinutes.bar.allHint")}
                disabled={!pmCandidates.candidates}
                onClick={() =>
                  setPmIds(eligibleIds(pmCandidates.candidates ?? []))
                }
              >
                {t("meetings.projectMinutes.bar.all")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                data-testid="pm-none"
                title={t("meetings.projectMinutes.bar.noneHint")}
                disabled={pmIds.length === 0}
                onClick={() => setPmIds([])}
              >
                {t("meetings.projectMinutes.bar.none")}
              </Button>
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <Button
              size="sm"
              data-testid="pm-start"
              disabled={pmIds.length < MIN_RECORDINGS}
              onClick={() => setPmDialog(true)}
            >
              <ListChecks width={14} height={14} />
              {t("meetings.projectMinutes.bar.start")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              data-testid="pm-cancel"
              onClick={() => {
                setPmMode(false);
                setPmIds([]);
              }}
            >
              {t("meetings.projectMinutes.bar.cancel")}
            </Button>
          </div>
          <p className="text-xs text-text/60" data-testid="pm-hint">
            {!pmCandidates.candidates
              ? t("meetings.projectMinutes.bar.loading")
              : pmCandidates.failed
                ? t("meetings.projectMinutes.bar.loadFailed")
                : pmIds.length < MIN_RECORDINGS
                  ? t("meetings.projectMinutes.bar.needTwo")
                  : " "}
          </p>
        </div>
      )}
      {selecting && (
        <div className="flex shrink-0 items-center justify-between gap-2 rounded-md bg-logo-primary/10 px-3 py-1.5">
          <span className="text-sm">
            {t("meetings.chat.list.selected", { count: selectedIds.length })}
          </span>
          <Button
            size="sm"
            onClick={askSelection}
            disabled={selectedIds.length === 0}
          >
            <MessageSquare width={14} height={14} />
            {t("meetings.chat.list.askSelection")}
          </Button>
        </div>
      )}
      {personFilter && (
        <div
          className="flex shrink-0 flex-wrap items-center gap-1.5"
          role="group"
          aria-label={t("meetings.people.filter.group")}
        >
          <span
            className="inline-flex items-center gap-1 rounded-full border border-logo-primary bg-logo-primary/20 px-2.5 py-0.5 text-xs text-text"
            data-testid="person-filter-chip"
          >
            {t("meetings.people.filter.chip", { name: personFilter.name })}
            <button
              type="button"
              onClick={() => onPersonFilterChange?.(null)}
              aria-label={t("meetings.people.filter.remove", {
                name: personFilter.name,
              })}
              title={t("meetings.people.filter.remove", {
                name: personFilter.name,
              })}
              className="rounded-full p-0.5 text-text/60 hover:bg-mid-gray/20 hover:text-text cursor-pointer"
              data-testid="person-filter-remove"
            >
              <X width={10} height={10} aria-hidden="true" />
            </button>
          </span>
        </div>
      )}
      {listError && <Alert variant="error">{listError}</Alert>}
      {deleteError && <Alert variant="error">{deleteError}</Alert>}

      <div
        data-testid="projects-scroll"
        className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden"
      >
        {searching ? (
          <>
            <p
              className="px-2 pb-1 text-xs text-text/60"
              data-testid="search-scope"
            >
              {t("meetings.projects.hitsIn", {
                count: total ?? items.length,
                name: selectionName,
              })}
            </p>
            {meetingsBlock(false)}
          </>
        ) : (
          <nav aria-label={t("meetings.projects.title")}>
            <ProjectRow
              id={ALL_PROJECTS}
              kind="all"
              name={t("meetings.projects.all")}
              count={counts.all}
              selected={selection === ALL_PROJECTS}
              open={listOpen}
              dropTarget={false}
              dropActive={false}
              onActivate={() => activate(ALL_PROJECTS)}
              onContextMenu={
                onNewMeeting
                  ? (x, y) => setPlainMenu({ x, y, id: ALL_PROJECTS })
                  : undefined
              }
            />
            {selection === ALL_PROJECTS && listOpen && meetingsBlock(true)}
            {folders.map((folder) => (
              <React.Fragment key={folder.id}>
                <ProjectRow
                  id={folder.id}
                  kind="project"
                  name={folder.name}
                  count={folder.meeting_count}
                  selected={selection === folder.id}
                  open={listOpen}
                  dropTarget
                  dropActive={dragging && dropId === folder.id}
                  editing={renamingId === folder.id}
                  onActivate={() => activate(folder.id)}
                  onRenameStart={() => {
                    setCreating(false);
                    setRenamingId(folder.id);
                  }}
                  onCommit={async (name) => {
                    const failed = await projects.rename(folder.id, name);
                    if (!failed) setRenamingId(null);
                    return failed;
                  }}
                  onCancel={() => setRenamingId(null)}
                  onMove={(dir) => void projects.move(folder.id, dir)}
                  onContextMenu={(x, y) =>
                    setProjectMenu({ x, y, id: folder.id })
                  }
                />
                {selection === folder.id && listOpen && (
                  <>
                    {!searching && onOpenProjectMinutes && (
                      <ProjectMinutesRows
                        items={pmList.items}
                        progress={pmRunProgress}
                        activeId={
                          activeProjectMinutes?.kind === "doc"
                            ? activeProjectMinutes.id
                            : null
                        }
                        runActive={
                          activeProjectMinutes?.kind === "run" &&
                          activeProjectMinutes.folderId === folder.id
                        }
                        onOpen={onOpenProjectMinutes}
                        onOpenRun={() => onOpenProjectRun?.(folder.id)}
                      />
                    )}
                    {meetingsBlock(true)}
                  </>
                )}
              </React.Fragment>
            ))}
            {creating && (
              <ProjectRow
                id="__new"
                kind="project"
                name=""
                count={null}
                selected={false}
                open={false}
                dropTarget={false}
                dropActive={false}
                editing
                onActivate={() => {}}
                onCommit={async (name) => {
                  const failed = await projects.create(name);
                  if (!failed) {
                    setCreating(false);
                    setListOpen(true);
                  }
                  return failed;
                }}
                onCancel={() => setCreating(false)}
              />
            )}
            <ProjectRow
              id={NO_PROJECT}
              kind="none"
              name={t("meetings.projects.none")}
              count={counts.none}
              selected={selection === NO_PROJECT}
              open={listOpen}
              dropTarget
              dropActive={dragging && dropId === NO_PROJECT}
              onActivate={() => activate(NO_PROJECT)}
              onContextMenu={
                onNewMeeting
                  ? (x, y) => setPlainMenu({ x, y, id: NO_PROJECT })
                  : undefined
              }
            />
            {selection === NO_PROJECT && listOpen && meetingsBlock(true)}
          </nav>
        )}
        <div ref={sentinelRef} className="h-1" />
      </div>

      <NextUp onPrepare={onPrepare} />

      {rowMenu && (
        <ContextMenu
          x={rowMenu.x}
          y={rowMenu.y}
          label={rowMenu.meeting.title}
          onClose={() => setRowMenu(null)}
          items={[
            // U7: eine wartende Datei umreihen oder herausnehmen.
            ...(() => {
              const place = queuePlace(queue, rowMenu.meeting.id);
              const id = rowMenu.meeting.id;
              return rowMenu.meeting.status === "queued" && place
                ? [
                    {
                      label: t("meetings.queue.toFront"),
                      disabled: place.position === 1,
                      onSelect: () => void commands.meetingsQueueToFront(id),
                    },
                    {
                      label: t("meetings.queue.remove"),
                      onSelect: () => void commands.meetingsQueueRemove(id),
                    },
                  ]
                : [];
            })(),
            {
              label: t("meetings.projects.moveTo"),
              onSelect: () => setPickerTarget(rowMenu.meeting),
            },
            {
              label: t("meetings.list.deleteButton"),
              danger: true,
              // Die laufende Aufnahme erst beenden: sonst schreibt das Backend
              // weiter in eine geloeschte Besprechung.
              disabled: rowMenu.meeting.id === liveId,
              onSelect: () => setDeleteTarget(rowMenu.meeting),
            },
          ]}
        />
      )}

      {projectMenu && projectMenuTarget && (
        <ContextMenu
          x={projectMenu.x}
          y={projectMenu.y}
          label={projectMenuTarget.name}
          onClose={() => setProjectMenu(null)}
          items={[
            ...(onNewMeeting
              ? [
                  {
                    label: t("meetings.projects.newMeetingHere"),
                    onSelect: () =>
                      onNewMeeting(projectMenuTarget.id, projectMenuTarget.id),
                  },
                ]
              : []),
            ...(onStartProjectMinutes
              ? [
                  {
                    label: t("meetings.projectMinutes.menu"),
                    onSelect: () => {
                      // Das Projekt waehlen; der Wechsel beendet eine laufende
                      // Auswahl, danach beginnt die neue.
                      setSelecting(false);
                      setPmIds([]);
                      if (folderId === projectMenuTarget.id) {
                        setPmMode(true);
                      } else {
                        pendingPmRef.current = projectMenuTarget.id;
                        projects.select(projectMenuTarget.id);
                      }
                    },
                  },
                ]
              : []),
            {
              label: t("meetings.projects.rename"),
              onSelect: () => {
                setCreating(false);
                setRenamingId(projectMenuTarget.id);
              },
            },
            ...(projectMenuIndex > 0
              ? [
                  {
                    label: t("meetings.projects.moveUp"),
                    onSelect: () =>
                      void projects.move(projectMenuTarget.id, -1),
                  },
                ]
              : []),
            ...(projectMenuIndex < folders.length - 1
              ? [
                  {
                    label: t("meetings.projects.moveDown"),
                    onSelect: () => void projects.move(projectMenuTarget.id, 1),
                  },
                ]
              : []),
            {
              label: t("meetings.projects.delete"),
              danger: true,
              onSelect: () => setDeleteProject(projectMenuTarget.id),
            },
          ]}
        />
      )}

      {plainMenu && onNewMeeting && (
        <ContextMenu
          x={plainMenu.x}
          y={plainMenu.y}
          label={
            plainMenu.id === ALL_PROJECTS
              ? t("meetings.projects.all")
              : t("meetings.projects.none")
          }
          onClose={() => setPlainMenu(null)}
          items={[
            {
              label: t("meetings.projects.newMeetingHere"),
              onSelect: () => onNewMeeting(null, plainMenu.id),
            },
          ]}
        />
      )}

      {onStartProjectMinutes && (
        <ProjectMinutesDialog
          open={pmDialog}
          count={pmIds.length}
          project={selectionName}
          onClose={() => setPmDialog(false)}
          onStart={(choice) => {
            if (!folderId) return;
            // In Listenreihenfolge (chronologisch ordnet das Backend ohnehin).
            const order = (pmCandidates.candidates ?? []).map(
              (c) => c.meeting_id,
            );
            const ids = order.filter((id) => pmIds.includes(id));
            setPmDialog(false);
            setPmMode(false);
            setPmIds([]);
            onStartProjectMinutes({
              folderId,
              meetingIds: ids.length > 0 ? ids : pmIds,
              templateId: choice.templateId,
              kind: choice.kind,
            });
          }}
        />
      )}

      <FolderPickerDialog
        meeting={pickerTarget}
        folders={folders}
        onClose={() => setPickerTarget(null)}
        onSaved={notifyMeetingsChanged}
      />

      <Dialog
        open={deleteProjectTarget !== undefined}
        onOpenChange={(o) => {
          if (!o) setDeleteProject(null);
        }}
        title={t("meetings.projects.deleteTitle")}
        closeLabel={t("meetings.folders.cancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteProject(null)}>
              {t("meetings.folders.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const id = deleteProject;
                setDeleteProject(null);
                if (id) void projects.remove(id);
              }}
            >
              {t("meetings.projects.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.projects.deleteBody", {
            name: deleteProjectTarget?.name ?? "",
          })}
        </p>
      </Dialog>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        title={t("meetings.list.deleteConfirmTitle")}
        closeLabel={t("meetings.list.cancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              {t("meetings.list.cancel")}
            </Button>
            <Button variant="danger" onClick={confirmDelete}>
              {t("meetings.list.deleteButton")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.list.deleteConfirm")}
        </p>
      </Dialog>
    </div>
  );
};
