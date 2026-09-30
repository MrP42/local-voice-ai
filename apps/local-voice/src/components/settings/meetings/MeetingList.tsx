import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  commands,
  events,
  type Folder,
  type Meeting,
  type MeetingSearchItem,
  type ScopeFilter,
} from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Alert } from "../../ui/Alert";
import Badge from "../../ui/Badge";
import {
  CheckSquare,
  FolderInput,
  MessageSquare,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { translateMeetingError } from "./meetingErrors";
import { SearchBar, SearchSnippet } from "./search/SearchBar";
import {
  EMPTY_FILTER,
  FilterChips,
  type ListFilter,
} from "./search/FilterChips";
import { ContextMenu, FolderChips } from "./search/FolderChips";
import type { PersonRef } from "./people/PersonPopover";
import { FolderPickerDialog } from "./search/FolderPickerDialog";
import { JobBar } from "./JobProgress";
import { useMeetingProgress } from "@/hooks/useMeetingJobs";

const PAGE_SIZE = 25;
const DAY_SECONDS = 86_400;

// One list for the picker filter AND the drag-and-drop filter — they must
// never diverge (same import pipeline behind both).
const IMPORT_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "mp4",
  "mkv",
  "mov",
  "flac",
  "ogg",
  "vtt",
  "srt",
];

const hasImportExtension = (path: string) => {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return IMPORT_EXTENSIONS.includes(ext);
};

// Windows paths use backslashes; the old class `[\/]` matched only the
// forward slash, so a C:\... path came back whole.
const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const statusBadgeVariant = (
  status: string,
): "primary" | "success" | "secondary" => {
  switch (status) {
    case "ready":
      return "success";
    case "recording":
      return "primary";
    default:
      return "secondary";
  }
};

const formatDuration = (durationMs: number | null) => {
  if (durationMs === null) return "--:--";
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

interface MeetingListProps {
  onSelect: (meeting: Meeting) => void;
  /** Die gewaehlte Besprechung: wird hervorgehoben, ihre Aenderungen (Titel,
      Status) landen sofort in der Zeile. */
  selected?: Meeting | null;
  /** Eine Besprechung wurde geloescht (die gewaehlte braucht dann Ersatz). */
  onDeleted?: (id: string) => void;
  /** M4-P4e: Chat ueber viele Besprechungen oeffnen (Scope vorbelegt). */
  onAsk?: (filter: ScopeFilter) => void;
  /** M5-P5d: Filter "Person: Anna Berg" (kommt aus dem Popover der Detailansicht). */
  personFilter?: PersonRef | null;
  onPersonFilterChange?: (person: PersonRef | null) => void;
}

export const MeetingList: React.FC<MeetingListProps> = ({
  onSelect,
  selected = null,
  onDeleted,
  onAsk,
  personFilter = null,
  onPersonFilterChange,
}) => {
  const { t, i18n } = useTranslation();
  // P8a: laufende Verarbeitungen (Fortschritt, Restdauer) statt nur "Wird verarbeitet".
  const progressMap = useMeetingProgress();
  // Ohne Suche/Filter tragen die Eintraege kein Snippet.
  const [items, setItems] = useState<MeetingSearchItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [hasMore, setHasMore] = useState(true);
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [importConsentPaths, setImportConsentPaths] = useState<string[] | null>(
    null,
  );
  const [isDragOver, setIsDragOver] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Meeting | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadingRef = useRef(false);
  // Jede Ladeanfrage bekommt eine Nummer; eine spaet eintreffende Antwort
  // einer ueberholten Suche darf die aktuelle Liste nicht ueberschreiben.
  const requestRef = useRef(0);

  // M4-P4d: Suche, Filter, Ordner
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ListFilter>(EMPTY_FILTER);
  const [folderId, setFolderId] = useState<string | null>(null);
  const [folders, setFolders] = useState<Folder[]>([]);
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
  const personId = personFilter?.id ?? null;
  const filtered =
    query !== "" ||
    folderId !== null ||
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
          more =
            pageItems.length > 0 &&
            offset + pageItems.length < (result.data?.total ?? 0);
          cut = result.data?.truncated ?? false;
        }
        setListError(null);
        setItems((prev) => (isFirstPage ? pageItems : [...prev, ...pageItems]));
        setHasMore(more);
        setTruncated(cut);
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

  // Andere Specs/aeltere Backends liefern hier `null`: dann eben keine Ordner.
  const loadFolders = useCallback(async () => {
    const result = await commands.meetingFoldersList();
    if (result.status === "ok") setFolders(result.data ?? []);
  }, []);

  useEffect(() => {
    void loadFolders();
  }, [loadFolders]);

  // Ein geloeschter Ordner darf nicht als aktiver Filter stehen bleiben.
  useEffect(() => {
    if (folderId !== null && !folders.some((f) => f.id === folderId)) {
      setFolderId(null);
    }
  }, [folders, folderId]);

  // Refresh from the current recording/import/generation lifecycle. The
  // import path in particular emits no state events at all — its command
  // result is the only signal — so this only covers the recording/generate
  // paths; import success triggers its own explicit reload below.
  useEffect(() => {
    const un = events.meetingEvent.listen((e) => {
      if (e.payload.kind === "state") void loadPage(0);
    });
    return () => {
      un.then((f) => f());
    };
  }, [loadPage]);

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

  const pickImportFile = async () => {
    setImportError(null);
    const picked = await open({
      multiple: false,
      filters: [
        {
          name: "Media",
          extensions: [
            "wav",
            "mp3",
            "m4a",
            "mp4",
            "mkv",
            "mov",
            "flac",
            "ogg",
            "vtt",
            "srt",
          ],
        },
      ],
    });
    if (typeof picked !== "string") return;
    // Spec A1: the import path needs the same consent confirmation as a
    // live recording — the file's mere existence is not proof that everyone
    // in it agreed to being recorded. The command is only ever called with
    // `consentConfirmed: true` after this dialog is explicitly confirmed.
    setImportConsentPaths([picked]);
  };

  // Drag-and-drop lands in the exact same consent-gated pipeline as the
  // picker button — dropping a file must not shortcut the Spec-A1 dialog.
  useEffect(() => {
    const un = getCurrentWebview().onDragDropEvent((event) => {
      const kind = event.payload.type;
      if (kind === "enter" || kind === "over") {
        setIsDragOver(true);
        return;
      }
      setIsDragOver(false);
      if (kind !== "drop") return;
      const accepted = event.payload.paths.filter(hasImportExtension);
      if (accepted.length === 0) {
        setImportError(t("meetings.errors.unsupportedFile"));
        return;
      }
      setImportError(null);
      setImportConsentPaths(accepted);
    });
    return () => {
      un.then((f) => f());
    };
  }, [t]);

  const confirmImport = async () => {
    const paths = importConsentPaths;
    if (!paths || paths.length === 0) return;
    setImportConsentPaths(null);
    setImporting(true);
    let lastError: string | null = null;
    for (const path of paths) {
      const result = await commands.meetingsImportFile(path, true);
      if (result.status === "error") {
        lastError = translateMeetingError(result.error, t);
      }
      // Refresh after every file so long batches show progress in the list.
      // (The synchronous VTT/SRT path emits no state events — the command
      // return is its only signal.)
      void loadPage(0);
    }
    setImporting(false);
    if (lastError) setImportError(lastError);
  };

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
    // Die Ordnerzaehler zaehlen nur lebende Besprechungen.
    void loadFolders();
  };

  // Ordner und Zeitraum der Liste gelten auch fuer den Chat; Quelle und
  // "mit Notizen" kennt der Chat-Scope nicht.
  const askAll = () =>
    onAsk?.({
      meeting_ids: null,
      folder_id: folderId,
      person: null,
      person_id: personId,
      from: fromTs,
      to: null,
      event_uid: null,
    });

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

  const afterFoldersChanged = () => {
    void loadFolders();
    // Im Ordnerfilter kann die Besprechung gerade herausgefallen sein.
    if (folderId !== null) void loadPage(0);
  };

  return (
    <SettingsGroup title={t("meetings.list.title")}>
      <div
        className={`px-4 py-3 space-y-3 rounded-md transition-colors ${
          isDragOver
            ? "outline-2 outline-dashed outline-logo-primary bg-logo-primary/5"
            : ""
        }`}
      >
        <div className="flex flex-wrap justify-between items-center gap-2">
          <p className="text-sm text-text/70">{t("meetings.list.title")}</p>
          <div className="flex flex-wrap items-center justify-end gap-2">
            {onAsk && (
              <>
                <Button variant="secondary" size="sm" onClick={askAll}>
                  <MessageSquare width={14} height={14} />
                  {t("meetings.chat.list.askAll")}
                </Button>
                <Button
                  variant={selecting ? "primary-soft" : "secondary"}
                  size="sm"
                  aria-pressed={selecting}
                  onClick={() => {
                    setSelecting((on) => !on);
                    setSelectedIds([]);
                  }}
                >
                  <CheckSquare width={14} height={14} />
                  {selecting
                    ? t("meetings.chat.list.selectDone")
                    : t("meetings.chat.list.select")}
                </Button>
              </>
            )}
            <Button
              variant="secondary"
              size="sm"
              onClick={pickImportFile}
              disabled={importing}
            >
              <Upload width={14} height={14} />
              {t("meetings.list.import")}
            </Button>
          </div>
        </div>
        {selecting && (
          <div className="flex items-center justify-between gap-2 rounded-md bg-logo-primary/10 px-3 py-1.5">
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
        <div className="space-y-2">
          <SearchBar onSearch={setQuery} />
          <FolderChips
            folders={folders}
            activeId={folderId}
            onSelect={setFolderId}
            onChanged={afterFoldersChanged}
          />
          <FilterChips value={filter} onChange={setFilter} />
          {personFilter && (
            <div
              className="flex flex-wrap items-center gap-1.5"
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
        </div>
        {isDragOver && (
          <p className="text-sm text-logo-primary font-medium text-center">
            {t("meetings.list.dropHint")}
          </p>
        )}
        {listError && <Alert variant="error">{listError}</Alert>}
        {importError && <Alert variant="error">{importError}</Alert>}
        {deleteError && <Alert variant="error">{deleteError}</Alert>}

        {loading ? (
          <p className="text-sm text-text/60 text-center py-3">
            {t("meetings.list.loading")}
          </p>
        ) : items.length === 0 ? (
          <p className="text-sm text-text/60 text-center py-3">
            {query !== ""
              ? t("meetings.search.noResults")
              : filtered
                ? t("meetings.search.noFilterResults")
                : t("meetings.list.empty")}
          </p>
        ) : (
          <div className="divide-y divide-mid-gray/20">
            {truncated && (
              <p className="text-xs text-text/60 py-1">
                {t("meetings.search.truncated")}
              </p>
            )}
            {items.map(({ meeting, snippet, hit_source }) => {
              const timestamp = meeting.started_at ?? meeting.created_at;
              const dateLabel = new Intl.DateTimeFormat(i18n.language, {
                year: "numeric",
                month: "short",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
              }).format(new Date(timestamp * 1000));

              return (
                <div
                  key={meeting.id}
                  className={`flex flex-wrap items-center justify-between gap-2 py-2 cursor-pointer hover:bg-mid-gray/10 rounded-md px-1 ${
                    selected?.id === meeting.id ? "bg-logo-primary/15" : ""
                  }`}
                  data-meeting-id={meeting.id}
                  aria-current={
                    selected?.id === meeting.id ? "true" : undefined
                  }
                  onClick={() =>
                    selecting ? toggleSelected(meeting.id) : onSelect(meeting)
                  }
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setRowMenu({ x: e.clientX, y: e.clientY, meeting });
                  }}
                >
                  {selecting && (
                    <input
                      type="checkbox"
                      className="shrink-0 cursor-pointer"
                      aria-label={t("meetings.chat.list.selectRow", {
                        title: meeting.title,
                      })}
                      checked={selectedIds.includes(meeting.id)}
                      onClick={(e) => e.stopPropagation()}
                      onChange={() => toggleSelected(meeting.id)}
                    />
                  )}
                  <div className="min-w-[9rem] flex-1">
                    <p className="text-sm font-medium truncate">
                      {meeting.title}
                    </p>
                    {/* The file an import came from, kept visible even after
                        the title was renamed away from it. */}
                    {meeting.source_path && (
                      <p
                        className="text-xs text-text/50 truncate"
                        title={meeting.source_path}
                      >
                        {baseName(meeting.source_path)}
                      </p>
                    )}
                    <p className="text-xs text-text/60">
                      {dateLabel} · {formatDuration(meeting.duration_ms)}
                      {hit_source && (
                        <>
                          {" · "}
                          {t("meetings.search.hitIn", {
                            source: t(
                              `meetings.search.hitSource.${hit_source}`,
                            ),
                          })}
                        </>
                      )}
                    </p>
                    {snippet && <SearchSnippet snippet={snippet} />}
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    {progressMap[meeting.id] &&
                    (meeting.status === "processing" ||
                      meeting.status === "recording") ? (
                      <JobBar progress={progressMap[meeting.id]} />
                    ) : (
                      <Badge variant={statusBadgeVariant(meeting.status)}>
                        {t(`meetings.status.${meeting.status}`, {
                          defaultValue: meeting.status,
                        })}
                      </Badge>
                    )}
                    <button
                      type="button"
                      className="p-1.5 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/15 cursor-pointer"
                      title={t("meetings.folders.moveTo")}
                      aria-label={t("meetings.folders.moveTo")}
                      onClick={(e) => {
                        e.stopPropagation();
                        setPickerTarget(meeting);
                      }}
                    >
                      <FolderInput width={16} height={16} />
                    </button>
                    <button
                      type="button"
                      className="p-1.5 rounded-md text-text/50 hover:text-red-400 hover:bg-red-500/10 cursor-pointer"
                      title={t("meetings.list.deleteButton")}
                      onClick={(e) => {
                        e.stopPropagation();
                        setDeleteTarget(meeting);
                      }}
                    >
                      <Trash2 width={16} height={16} />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
        <div ref={sentinelRef} className="h-1" />
      </div>

      {rowMenu && (
        <ContextMenu
          x={rowMenu.x}
          y={rowMenu.y}
          label={rowMenu.meeting.title}
          onClose={() => setRowMenu(null)}
          items={[
            {
              label: t("meetings.folders.moveTo"),
              onSelect: () => setPickerTarget(rowMenu.meeting),
            },
            {
              label: t("meetings.list.deleteButton"),
              danger: true,
              onSelect: () => setDeleteTarget(rowMenu.meeting),
            },
          ]}
        />
      )}

      <FolderPickerDialog
        meeting={pickerTarget}
        folders={folders}
        onClose={() => setPickerTarget(null)}
        onSaved={afterFoldersChanged}
      />

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

      <Dialog
        open={importConsentPaths !== null}
        onOpenChange={(open) => {
          if (!open) setImportConsentPaths(null);
        }}
        title={t("meetings.consent.title")}
        closeLabel={t("meetings.consent.cancel")}
        footer={
          <>
            <Button
              variant="secondary"
              onClick={() => setImportConsentPaths(null)}
            >
              {t("meetings.consent.cancel")}
            </Button>
            <Button onClick={confirmImport} disabled={importing}>
              {t("meetings.consent.confirm")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80 whitespace-pre-wrap">
          {t("meetings.consent.importBody")}
        </p>
        {importConsentPaths && (
          <ul className="mt-2 text-xs text-text/60 space-y-0.5">
            {importConsentPaths.map((p) => (
              <li key={p} className="truncate">
                {baseName(p)}
              </li>
            ))}
          </ul>
        )}
      </Dialog>
    </SettingsGroup>
  );
};
