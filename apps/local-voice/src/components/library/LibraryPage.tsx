import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSectionActive } from "../shell/KeepAlive";
import {
  BookOpen,
  CheckCircle2,
  CheckSquare,
  Download,
  Eraser,
  FileText,
  Languages,
  Loader2,
  Headphones,
  ImageIcon,
  Layers,
  MoreVertical,
  Pencil,
  Play,
  Plus,
  Sparkles,
  Star,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import type { LibraryBook } from "@/bindings";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { ActionMenu, type ActionMenuItem } from "../ui/ActionMenu";
import { FormatRows, FormatValue, formatEntries } from "./FormatRows";
import { useKurzbuecher } from "./useKurzbuecher";
import { Select } from "../ui/Select";
import { BookCover } from "./BookCover";
import { BookDetail } from "./BookDetail";
import { AddBookDialog } from "./AddBookDialog";
import { AudioBar } from "./AudioBar";
import { AlignDialog } from "./AlignDialog";
import { LangBadges } from "./LangBadges";
import {
  PairsBar,
  PairsDialog,
  joinPairs,
  reportJoined,
  useLanguagePairs,
  type ResolvedPair,
} from "./LanguagePairs";
import { useEditionOffer } from "./EditionOffer";
import { usePlayerLanguage, type Listening } from "./usePlayerLanguage";
import { MergeDialog } from "./MergeDialog";
import { RemoveDialog } from "./RemoveDialog";
import { CleanTitlesDialog } from "./CleanTitlesDialog";
import { CoverDialog } from "./CoverDialog";
import { IconButton } from "../ui/IconButton";
import { useLibraryDrop } from "./useLibraryDrop";
import { useLibrary, type TranscriptJob } from "./useLibrary";
import {
  formatDuration,
  isFavorite,
  isFinished,
  isStarted,
  matchesFilter,
  matchesSearch,
  openInReader,
  openKiDocInReader,
  sortViews,
  viewOf,
  type BookView,
  type Filter,
  type Sort,
  tracksOf,
} from "@/lib/library/books";
import { flushProgress, setFinished } from "@/lib/library/progress";
import {
  audioKeyOf,
  editionBook,
  readingEdition,
  textKeyOf,
} from "@/lib/library/editions";
import {
  splitAudiobookGroups,
  type SplitGroup,
} from "@/lib/library/audioGroups";
import {
  mergeGroup,
  planManual,
  planMerge,
  reorder,
  type MergePlan,
} from "@/lib/library/merge";
import {
  deleteBooks,
  listTrash,
  purgeBooks,
  purgeStaleTrash,
  restoreBooks,
  trashBooks,
  UNDO_MS,
  type RemoveScope,
} from "@/lib/library/remove";
import type { TrashEntry } from "@/bindings";
import { TrashDialog } from "./TrashDialog";
import {
  KiCreateDialog,
  ensureBookJobToasts,
  jobStatus,
  jobTitle,
  type KiCreateTab,
} from "./KiCreateDialog";
import { ExportDialog } from "./ExportDialog";
import {
  cancelBookJob,
  jobFraction,
  jobsOfBook,
  useBookJobs,
} from "@/lib/library/bookJobs";
import "./fassungen.css";
import { cleanTitle, hasTitleNoise } from "@/lib/library/names";
import { commands } from "@/bindings";
import { usePersistentState } from "@/hooks/usePersistentState";
import "./library.css";

const FILTERS: Filter[] = [
  "all",
  "favorites",
  "notStarted",
  "inProgress",
  "finished",
  "noAudio",
  "working",
];
const SORTS: Sort[] = ["recent", "added", "title", "author"];
const isSort = (v: string) => (SORTS as string[]).includes(v);
const isFilter = (v: string) => (FILTERS as string[]).includes(v);

/** Mindestbreite einer „Weitermachen“-Kachel samt Abstand (library.css). */
const CONTINUE_MIN = 220;
const CONTINUE_GAP = 12;

/**
 * Wie viele „Weitermachen“-Kacheln passen (Patrick 04.11.: Platz nutzen,
 * mindestens vier). Breit: eine volle Reihe; mittel: zwei Reihen;
 * schmal (eine Spalte): vier untereinander.
 */
function useContinueSlots(el: HTMLDivElement | null): number {
  const [slots, setSlots] = useState(4);
  useEffect(() => {
    if (!el || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      const width = el.clientWidth;
      if (!width) return;
      const cols = Math.max(
        1,
        Math.floor((width + CONTINUE_GAP) / (CONTINUE_MIN + CONTINUE_GAP)),
      );
      setSlots(
        cols >= 4 ? Math.min(cols, 8) : cols === 1 ? 4 : Math.min(cols * 2, 6),
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return slots;
}

/** Text zum Stand der Transkription: bei mehreren Aufträgen „12 von 36 · 4 parallel“. */
export function jobLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  job: TranscriptJob | undefined,
): string {
  if (job && job.total > 1 && job.failed > 0 && job.failed < job.total)
    return t("library.job.partsFailed", {
      done: job.done,
      total: job.total,
      failed: job.failed,
    });
  if (job && job.total > 1 && job.status !== "failed")
    return t(job.running > 1 ? "library.job.partsParallel" : "library.job.parts", {
      done: job.done,
      total: job.total,
      running: job.running,
    });
  return t(`library.job.${job?.status ?? "queued"}`, {
    defaultValue: t("library.job.processing"),
  });
}

export const Formats: React.FC<{ view: BookView; job?: TranscriptJob }> = ({
  view,
  job,
}) => {
  const { t } = useTranslation();
  return (
    <span className="library-fmt">
      <span data-state={view.text} title={t(`library.text.${view.text}`)}>
        <BookOpen size={12} aria-hidden="true" />
        {view.text === "busy"
          ? jobLabel(t, job)
          : t(`library.textShort.${view.text}`)}
      </span>
      <span
        data-state={view.audio}
        title={t(`library.audioSource.${view.audioSource ?? "none"}`)}
      >
        <Headphones size={12} aria-hidden="true" />
        {t(`library.audioShort.${view.audioSource ?? "none"}`)}
      </span>
    </span>
  );
};

export const ProgressBar: React.FC<{ fraction: number; label: string }> = ({
  fraction,
  label,
}) => (
  <div
    className="library-progress"
    role="progressbar"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(fraction * 100)}
  >
    <span style={{ width: `${Math.round(fraction * 100)}%` }} />
  </div>
);

/**
 * Bibliothek (lva#152, Codex-Wahl 01.11.): Startansicht als Raster mit
 * „Weiterhören“ (Variante A), kompakte Kachel „In Arbeit“ (aus C), Klick auf
 * ein Cover öffnet die Produktseite (D). Ein Buch ist genau ein Eintrag mit
 * Text- und/oder Audiofassung.
 */
export const LibraryPage: React.FC = () => {
  const { t } = useTranslation();
  const { books, pageAudio, jobs, reload, save, tick } = useLibrary();
  const {
    drop,
    working,
    dialog: dropDialog,
    importPaths,
  } = useLibraryDrop(books, reload, (book) => offerFor(book));
  const [query, setQuery] = useState("");
  const [filter, setFilter] = usePersistentState<Filter>(
    "library.filter",
    "all",
    isFilter,
  );
  const [sort, setSort] = usePersistentState<Sort>(
    "library.sort",
    "recent",
    isSort,
  );
  const [openId, setOpenId] = useState<string | null>(null);
  /** ⋮ → „Titel bearbeiten“: Produktseite öffnet mit dem Titelfeld. */
  const [editTitleId, setEditTitleId] = useState<string | null>(null);
  const [coverFor, setCoverFor] = useState<LibraryBook | null>(null);
  /** lva#236: KI-Aufträge und Export direkt aus der Bibliothek. */
  const [kiFor, setKiFor] = useState<{ book: LibraryBook; tab: KiCreateTab } | null>(null);
  const [exportFor, setExportFor] = useState<LibraryBook | null>(null);
  const kiJobs = useBookJobs();
  const kurzbuecher = useKurzbuecher(books);
  const [continueEl, setContinueEl] = useState<HTMLDivElement | null>(null);
  useEffect(() => ensureBookJobToasts(), []);
  useEffect(() => {
    if (!openId) setEditTitleId(null);
  }, [openId]);
  const [adding, setAdding] = useState(false);
  const [listening, setListening] = useState<Listening | null>(null);
  /** „Kapitel zuordnen“ (sf-08) für ein Paar von Sprachfassungen. */
  const [aligning, setAligning] = useState<{
    bookId: string;
    pair: { a: string; b: string };
  } | null>(null);
  const { offerFor, dialog: offerDialog } = useEditionOffer(reload);
  const languagePairs = useLanguagePairs(books);
  const [pairsOpen, setPairsOpen] = useState(false);
  const [pairsBusy, setPairsBusy] = useState(false);
  const joinChosen = async (chosen: ResolvedPair[]) => {
    setPairsBusy(true);
    try {
      reportJoined(t, await joinPairs(chosen));
    } finally {
      setPairsBusy(false);
      setPairsOpen(false);
      await reload();
    }
  };
  /** Höhe der Hörleiste samt Lesefläche: die Seite lässt unten so viel Platz. */
  const [playerHeight, setPlayerHeight] = useState(0);
  const [merging, setMerging] = useState<{
    group: SplitGroup;
    plan: MergePlan | null;
    busy: boolean;
  } | null>(null);
  const [dismissed, setDismissed] = useState<string[]>(() => {
    try {
      const raw = window.localStorage.getItem("lva.ui.library.mergeDismissed");
      const list: unknown = raw ? JSON.parse(raw) : [];
      return Array.isArray(list)
        ? list.filter((x) => typeof x === "string")
        : [];
    } catch {
      return [];
    }
  });
  const listen = (book: LibraryBook, follow = false) =>
    setListening({
      book,
      follow,
      edition: readingEdition(book)?.id ?? null,
      startPlaying: true,
      focusLanguage: false,
    });

  // Mehrfachauswahl: Zusammenführen, Titel bereinigen, Entfernen.
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [removing, setRemoving] = useState<LibraryBook[]>([]);
  // Erhalten beim Bereichswechsel (KeepAlive): beim Zurückkommen still
  // auffrischen (Fortschritt, neue Kurzbücher), ohne die Seite zu leeren.
  const sectionActive = useSectionActive();
  const wasActive = useRef(sectionActive);
  useEffect(() => {
    if (sectionActive && !wasActive.current) void reload();
    wasActive.current = sectionActive;
    // Verdeckt: Dialoge schließen (sie liegen im Portal am body und blieben
    // sonst über dem anderen Bereich stehen).
    if (!sectionActive) {
      setAdding(false);
      setMerging(null);
      setRemoving([]);
      setCoverFor(null);
    }
  }, [sectionActive, reload]);
  const [cleaning, setCleaning] = useState<LibraryBook[]>([]);
  const [cleanBusy, setCleanBusy] = useState(false);
  /** Papierkorb: dauerhaft erreichbar, 30 Tage wiederherstellbar. */
  const [trash, setTrash] = useState<TrashEntry[]>([]);
  const [trashOpen, setTrashOpen] = useState(false);
  /** Meldung im offenen Papierkorb statt eines Hinweises darüber (lva#221). */
  const [trashNotice, setTrashNotice] = useState<{ text: string; error?: boolean } | null>(null);
  const trashOpenRef = useRef(false);
  trashOpenRef.current = trashOpen;
  /** Hinweis unten rechts, solange der Papierkorb zu ist; sonst Meldung im Dialog. */
  const notify = (text: string, error = false) => {
    if (trashOpenRef.current) setTrashNotice({ text, error });
    else if (error) toast.error(text);
    else toast(text);
  };
  const [trashBusy, setTrashBusy] = useState(false);
  const refreshTrash = async () => setTrash(await listTrash());
  useEffect(() => {
    void purgeStaleTrash().finally(() => void refreshTrash());
  }, []);
  const toggleSelected = (id: string) =>
    setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
  const endSelection = () => {
    setSelecting(false);
    setSelected([]);
  };
  const selectedBooks = (books ?? []).filter((b) => selected.includes(b.id));
  // Nur Sichtbares bleibt ausgewählt (Suche oder Filter geändert).
  const noisy = useMemo(
    () => (books ?? []).filter((b) => hasTitleNoise(b.title)),
    [books],
  );

  const views = useMemo(
    () =>
      (books ?? []).map((b) =>
        viewOf(
          b,
          (() => {
            const page = readingEdition(b)?.text?.page_id ?? b.text?.page_id;
            return page ? !!pageAudio[page] : false;
          })(),
        ),
      ),
    // tick: Fortschritt aus dem localStorage neu lesen.
    [books, pageAudio, tick],
  );
  const splitGroups = useMemo(
    () =>
      splitAudiobookGroups(books ?? []).filter(
        (g) => !dismissed.includes(g.key),
      ),
    [books, dismissed],
  );
  const dismissGroup = (key: string) => {
    const next = [...dismissed, key];
    setDismissed(next);
    try {
      window.localStorage.setItem(
        "lva.ui.library.mergeDismissed",
        JSON.stringify(next),
      );
    } catch {
      /* ohne Speicher kommt der Hinweis beim nächsten Start wieder */
    }
  };
  const askMerge = async (group: SplitGroup) => {
    setMerging({ group, plan: null, busy: false });
    try {
      const plan = await planMerge(group);
      setMerging({ group, plan, busy: false });
    } catch (e) {
      setMerging(null);
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };
  const askManualMerge = (list: LibraryBook[]) => {
    try {
      const { group, plan } = planManual(list);
      setMerging({ group, plan, busy: false });
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };
  const moveMergePart = (from: number, to: number) => {
    if (!merging?.plan || merging.busy) return;
    const next = reorder(merging.group, merging.plan, from, to);
    setMerging({ ...merging, ...next });
  };
  /** Hörleiste schließen, wenn sie aus einem der Bücher spielt. */
  const releasePlayer = async (ids: Set<string>) => {
    if (!listening || !ids.has(listening.book.id)) return;
    setListening(null);
    await new Promise((resolve) => window.setTimeout(resolve, 60));
    await flushProgress();
  };
  const doRemove = async (scope: RemoveScope) => {
    const list = removing;
    setRemoving([]);
    if (!list.length) return;
    const ids = new Set(list.map((b) => b.id));
    await releasePlayer(ids);
    if (openId && ids.has(openId)) setOpenId(null);
    try {
      if (scope === "delete") await deleteBooks(list);
      else await trashBooks(list);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
      await Promise.all([reload(), refreshTrash()]);
      return;
    }
    endSelection();
    await Promise.all([reload(), refreshTrash()]);
    if (scope === "delete") {
      toast(
        t("library.remove.deleted", {
          count: list.length,
          title: list[0].title,
        }),
      );
      return;
    }
    toast(
      list.length === 1
        ? t("library.remove.doneOne", { title: list[0].title })
        : t("library.remove.doneMany", { count: list.length }),
      {
        className: "library-undo-toast",
        description: t("library.remove.doneHint"),
        duration: UNDO_MS,
        action: {
          label: t("library.remove.undo"),
          onClick: () => void restoreFromTrash(list),
        },
      },
    );
  };
  const restoreFromTrash = async (list: LibraryBook[]) => {
    setTrashBusy(true);
    try {
      await restoreBooks(list);
      notify(
        t("library.remove.restored", {
          count: list.length,
          title: list[0]?.title ?? "",
        }),
      );
    } catch (e) {
      notify(e instanceof Error ? e.message : String(e), true);
    } finally {
      setTrashBusy(false);
      await Promise.all([reload(), refreshTrash()]);
    }
  };
  const purgeFromTrash = async (list: LibraryBook[]) => {
    setTrashBusy(true);
    try {
      await purgeBooks(list);
      notify(
        t("library.remove.deleted", {
          count: list.length,
          title: list[0]?.title ?? "",
        }),
      );
    } catch (e) {
      notify(e instanceof Error ? e.message : String(e), true);
    } finally {
      setTrashBusy(false);
      await refreshTrash();
    }
  };
  const doClean = async () => {
    setCleanBusy(true);
    try {
      // Frisch lesen und nur den Titel ändern: eine inzwischen fertige
      // Transkription bleibt erhalten.
      for (const book of cleaning)
        await save(book.id, (fresh) => ({
          ...fresh,
          title: cleanTitle(fresh.title),
        }));
      toast(t("library.clean.done", { count: cleaning.length }));
      setCleaning([]);
      endSelection();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setCleanBusy(false);
      await reload();
    }
  };
  const doMerge = async () => {
    if (!merging?.plan) return;
    const { group, plan } = merging;
    setMerging({ ...merging, busy: true });
    // Die Hörleiste schließt vorher: sie sichert dabei ihre Stelle, und beim
    // Verschieben der Dateien spielt nichts mehr aus ihnen.
    const involved = new Set([plan.target.id, ...group.books.map((b) => b.id)]);
    if (listening && involved.has(listening.book.id)) {
      setListening(null);
      // Zeit zum Abmelden der Leiste (sie sichert beim Schließen ihre Stelle).
      await new Promise((resolve) => window.setTimeout(resolve, 60));
      await flushProgress();
    }
    try {
      const book = await mergeGroup(group, plan);
      toast(
        t("library.merge.done", {
          title: book.title,
          count: group.books.length,
        }),
      );
      setMerging(null);
      endSelection();
      await reload();
    } catch (e) {
      setMerging(null);
      toast.error(e instanceof Error ? e.message : String(e));
      await reload();
    }
  };

  const {
    playing,
    edition: playingEdition,
    loading: loadingEditions,
    switchTo,
  } = usePlayerLanguage({
    books,
    listening,
    setListening,
    reload,
    openAlign: (bookId, pair) => setAligning({ bookId, pair }),
  });
  const counts = useMemo(
    () =>
      Object.fromEntries(
        FILTERS.map((f) => [
          f,
          views.filter((v) => matchesFilter(v, f)).length,
        ]),
      ) as Record<Filter, number>,
    [views],
  );
  const shown = sortViews(
    views.filter((v) => matchesFilter(v, filter) && matchesSearch(v, query)),
    sort,
  );
  const shownKey = shown.map((v) => v.book.id).join(",");
  useEffect(() => {
    const visible = new Set(shownKey.split(","));
    setSelected((s) => {
      const next = s.filter((id) => visible.has(id));
      return next.length === s.length ? s : next;
    });
  }, [shownKey]);
  const continueSlots = useContinueSlots(continueEl);
  const continueViews = sortViews(views.filter(isStarted), "recent", {
    favoritesFirst: false,
  }).slice(0, continueSlots);
  const jobList = Object.values(jobs);
  const kiJobList = kiJobs.filter((j) => books?.some((b) => b.id === j.bookId));
  const hasWork = jobList.length > 0 || kiJobList.length > 0;

  const jobOf = (book: LibraryBook) => jobs[book.id];

  /**
   * Sofort weiter, nur mit vorhandenem Audio: Hörbuch in der Hörleiste,
   * vertonte Textfassung im Reader mit Autostart. Ohne Audio gibt es kein
   * Abspielen, sondern Lesen.
   */
  const play = (v: BookView) => {
    if (
      v.shown.audio_path &&
      (v.resumeWith === "audio" ||
        (v.audioSource === "audiobook" && !v.shown.text))
    ) {
      listen(v.book);
      return;
    }
    if (v.shown.text && v.audioSource === "narrated")
      openInReader(v.shown.text.page_id, true);
    else if (v.shown.audio_path) listen(v.book);
  };

  const toggleFinished = async (v: BookView) => {
    const finished = !isFinished(v);
    const ed = readingEdition(v.book);
    if (v.shown.text)
      setFinished(
        (ed && textKeyOf(ed)) || `text:${v.shown.text.page_id}:original`,
        finished,
      );
    if (v.shown.audio_path)
      setFinished(ed ? audioKeyOf(v.book, ed) : `audio:${v.book.id}`, finished);
    await save(v.book.id, (fresh) => ({ ...fresh, finished }));
  };

  const toggleFavorite = async (v: BookView) => {
    try {
      // Vom frischen Stand umschalten: ein Doppelklick vor dem Neuladen hebt sich auf.
      await save(v.book.id, (fresh) => ({ ...fresh, favorite: !fresh.favorite }));
    } catch (e) {
      toast.error(t("library.favorite.failed", { error: String(e) }));
    }
  };

  const open = openId ? views.find((v) => v.book.id === openId) : undefined;

  const playButton = (v: BookView, className = "") =>
    !v.audioSource ? (
      v.shown.text ? (
        <button
          type="button"
          className={`library-play library-play--read ${className}`}
          onClick={(e) => {
            e.stopPropagation();
            openInReader(v.shown.text!.page_id, false);
          }}
          aria-label={t("library.readTitle", { title: v.book.title })}
          title={t("library.read")}
          data-testid="library-read-quick"
        >
          <BookOpen size={18} aria-hidden="true" />
        </button>
      ) : null
    ) : (
      <button
        type="button"
        className={`library-play ${className}`}
        onClick={(e) => {
          e.stopPropagation();
          play(v);
        }}
        aria-label={t("library.playTitle", { title: v.book.title })}
        title={t("library.play")}
        data-testid="library-play"
      >
        <Play size={18} fill="currentColor" aria-hidden="true" />
      </button>
    );

  const statusLine = (v: BookView) => {
    if (isFinished(v)) return t("library.status.finished");
    if (v.fraction > 0) {
      const rec = v.progress;
      if (rec?.unit === "ms" && rec.length > 0)
        return t("library.status.left", {
          time: formatDuration(rec.length - rec.offset),
        });
      return t("library.status.percent", {
        percent: Math.round(v.fraction * 100),
      });
    }
    return v.shown.text || v.shown.audio_path
      ? t("library.status.notStarted")
      : t("library.status.empty");
  };

  /**
   * Ein Menü je Karte (Patrick 04.11.): Öffnen, KI und Export, Verwalten.
   * Das frühere KI-Menü ist hier aufgegangen; Lesen und Hören starten über die
   * Fassungszeilen der Karte.
   */
  const moreItems = (v: BookView): ActionMenuItem[] => {
    const kb = kurzbuecher[v.book.id];
    return [
      {
        id: "details",
        label: t("library.details"),
        icon: BookOpen,
        onSelect: () => setOpenId(v.book.id),
      },
      ...(v.shown.audio_path && v.shown.text
        ? [
            {
              id: "listen-follow",
              label: t("library.listenFollow"),
              icon: BookOpen,
              onSelect: () => listen(v.book, true),
            },
          ]
        : []),
      ...(kb
        ? [
            {
              id: "kurzbuch-open",
              label: t("library.fmt.kurzbuchOpen"),
              icon: Sparkles,
              testId: `library-kurzbuch-open-${v.book.id}`,
              onSelect: () =>
                openKiDocInReader({
                  page: kb.page,
                  doc: kb.doc.id,
                  play: false,
                }),
            },
          ]
        : []),
      {
        id: "translate",
        label: t("library.ki.menuTranslate"),
        icon: Languages,
        dividerBefore: true,
        testId: `library-ki-translate-${v.book.id}`,
        onSelect: () => setKiFor({ book: v.book, tab: "translation" }),
      },
      {
        id: "kurzbuch",
        label: t("library.ki.menuKurzbuch"),
        icon: Sparkles,
        testId: `library-ki-kurzbuch-${v.book.id}`,
        onSelect: () => setKiFor({ book: v.book, tab: "kurzbuch" }),
      },
      {
        id: "kidoc",
        label: t("library.ki.menuKidoc"),
        icon: FileText,
        testId: `library-ki-kidoc-${v.book.id}`,
        onSelect: () => setKiFor({ book: v.book, tab: "kidoc" }),
      },
      {
        id: "export",
        label: t("library.export.menu"),
        icon: Download,
        testId: `library-export-${v.book.id}`,
        onSelect: () => setExportFor(v.book),
      },
      {
        id: "finished",
        label: isFinished(v)
          ? t("library.markUnfinished")
          : t("library.markFinished"),
        icon: CheckCircle2,
        dividerBefore: true,
        onSelect: () => void toggleFinished(v),
      },
      {
        id: "title",
        label: t("library.titleEdit.menu"),
        icon: Pencil,
        testId: `library-title-${v.book.id}`,
        onSelect: () => {
          setEditTitleId(v.book.id);
          setOpenId(v.book.id);
        },
      },
      {
        id: "cover",
        label: t("library.cover.menu"),
        icon: ImageIcon,
        testId: `library-cover-${v.book.id}`,
        onSelect: () => setCoverFor(v.book),
      },
      ...(hasTitleNoise(v.book.title)
        ? [
            {
              id: "clean",
              label: t("library.clean.one"),
              icon: Eraser,
              onSelect: () => setCleaning([v.book]),
            },
          ]
        : []),
      {
        id: "select",
        label: t("library.select.start"),
        icon: CheckSquare,
        onSelect: () => {
          setSelecting(true);
          setSelected([v.book.id]);
        },
      },
      {
        id: "remove",
        label: t("library.remove.action"),
        icon: Trash2,
        dividerBefore: true,
        testId: `library-remove-${v.book.id}`,
        onSelect: () => setRemoving([v.book]),
      },
    ];
  };

  const card = (v: BookView) => {
    const isSelected = selecting && selected.includes(v.book.id);
    return (
    <article
      className={`library-card${drop.targetId === v.book.id ? " library-card--drop" : ""}${isSelected ? " library-card--selected" : ""}`}
      key={v.book.id}
      data-testid="library-card"
      data-drop-book={v.book.id}
    >
      <div style={{ position: "relative" }}>
        {selecting && (
          <label className="library-select">
            <input
              type="checkbox"
              checked={isSelected}
              onChange={() => toggleSelected(v.book.id)}
              aria-label={t("library.select.one", { title: v.book.title })}
              data-testid="library-select"
            />
          </label>
        )}
        <button
          type="button"
          className="library-card__open"
          onClick={() =>
            selecting ? toggleSelected(v.book.id) : setOpenId(v.book.id)
          }
          aria-label={
            selecting
              ? t("library.select.one", { title: v.book.title })
              : t("library.openTitle", { title: v.book.title })
          }
          aria-pressed={selecting ? isSelected : undefined}
        >
          <BookCover book={v.book} />
        </button>
        {!selecting && playButton(v, "library-card__play")}
        {!selecting && (
        <button
          type="button"
          className="library-card__star"
          aria-pressed={isFavorite(v)}
          aria-label={t("library.favorite.toggle", { title: v.book.title })}
          title={t(isFavorite(v) ? "library.favorite.on" : "library.favorite.off")}
          onClick={() => void toggleFavorite(v)}
          data-testid={`library-star-${v.book.id}`}
        >
          <span className="library-card__star-dot" aria-hidden="true">
            <Star size={18} fill={isFavorite(v) ? "currentColor" : "none"} />
          </span>
        </button>
        )}
      </div>
      <div className="library-card__head">
        <div className="library-card__names">
          <div className="library-title" lang="de" title={v.book.title}>
            {v.book.title}
          </div>
          {v.book.authors.length > 0 && (
            <div className="library-meta library-card__author">
              {v.book.authors.join(", ")}
            </div>
          )}
          <LangBadges book={v.book} />
        </div>
        <span className="library-touch library-card__tools">
          <ActionMenu
            trigger={{
              icon: MoreVertical,
              label: t("library.more", { title: v.book.title }),
              description: t("library.moreHint"),
              testId: `library-more-${v.book.id}`,
              ghost: true,
            }}
            menuLabel={t("library.moreMenu")}
            align="end"
            widthClass="w-64 library-cardmenu"
            items={moreItems(v)}
          />
        </span>
      </div>
      <FormatRows
        view={v}
        busyText={v.text === "busy" ? jobLabel(t, jobOf(v.book)) : undefined}
        kurzbuch={kurzbuecher[v.book.id]}
        onListen={() => listen(v.book)}
      />
      {jobsOfBook(kiJobs, v.book.id).map((j) => (
        <div
          className="library-jobline"
          key={j.id}
          role="status"
          data-testid={`library-kijob-${v.book.id}`}
        >
          <Loader2 size={12} className="lib-spin" aria-hidden="true" />
          <span>
            {jobTitle(t, j)} · {jobStatus(t, j)}
          </span>
        </div>
      ))}
    </article>
    );
  };

  return (
    <PageShell
      title={t("library.title")}
      description={t("library.description")}
      actions={
        // Dezent (Patrick 03.11.): nur das Plus, Name als Tooltip.
        <IconButton
          icon={<Plus size={20} aria-hidden="true" />}
          label={t("library.add")}
          onClick={() => setAdding(true)}
          className="library-add-button"
          data-testid="library-add"
        />
      }
    >
      <div
        className={`library${drop.over ? " library--drop" : ""}`}
        data-library-drop=""
        style={
          {
            "--lib-bar": `${listening ? playerHeight || 64 : 0}px`,
          } as React.CSSProperties
        }
      >
        {(drop.over || working) && (
          <div
            className="library-drop-hint"
            role="status"
            data-testid="library-drop-hint"
          >
            {working
              ? t("library.drop.working", { name: working })
              : drop.targetId
                ? t("library.drop.onBook", {
                    title:
                      books?.find((b) => b.id === drop.targetId)?.title ?? "",
                  })
                : t("library.drop.onFree")}
          </div>
        )}
        {open ? (
          <div data-drop-book={open.book.id} style={{ display: "contents" }}>
            <BookDetail
              view={open}
              job={jobOf(open.book)}
              onBack={() => setOpenId(null)}
              onPlay={() => play(open)}
              onListen={() => listen(open.book)}
              onListenFollow={() => listen(open.book, true)}
              onToggleFinished={() => void toggleFinished(open)}
              onSave={save}
              onRemove={() => setRemoving([open.book])}
              onChanged={reload}
              editTitle={editTitleId === open.book.id}
              onCreate={(tab) => setKiFor({ book: open.book, tab })}
              onExport={() => setExportFor(open.book)}
              onAddEdition={() => setAdding(true)}
              onAlign={(pair) => setAligning({ bookId: open.book.id, pair })}
            />
          </div>
        ) : (
          <>
            <PairsBar
              pairs={languagePairs.later ? [] : languagePairs.pairs}
              busy={pairsBusy}
              onJoin={(p) => void joinChosen([p])}
              onReview={() => setPairsOpen(true)}
              onLater={languagePairs.postpone}
            />
            <div className="library-bar">
              <div className="library-bar__search">
                <Input
                  type="search"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={t("library.search")}
                  aria-label={t("library.search")}
                />
              </div>
              <span className="library-bar__spacer" />
              {!selecting && (books?.length ?? 0) > 0 && (
                <Button
                  icon={<CheckSquare size={16} aria-hidden="true" />}
                  onClick={() => setSelecting(true)}
                  data-testid="library-select-start"
                >
                  {t("library.select.start")}
                </Button>
              )}
              {trash.length > 0 && (
                <Button
                  icon={<Trash2 size={16} aria-hidden="true" />}
                  onClick={() => {
                    // Der Papierkorb ersetzt den Rückgängig-Hinweis; keine Überlagerung.
                    toast.dismiss();
                    setTrashNotice(null);
                    setTrashOpen(true);
                  }}
                  data-testid="library-trash-open"
                >
                  {t("library.trash.open", { count: trash.length })}
                </Button>
              )}
              <div className="library-sort" data-testid="library-sort">
                <Select
                  value={sort}
                  isClearable={false}
                  ariaLabel={t("library.sortLabel")}
                  options={SORTS.map((s) => ({
                    value: s,
                    label: t(`library.sort.${s}`),
                  }))}
                  onChange={(v) => v && setSort(v as Sort)}
                />
              </div>
              <div
                className="library-chips"
                role="group"
                aria-label={t("library.filterLabel")}
              >
                {FILTERS.map((f) => (
                  <button
                    key={f}
                    type="button"
                    className="library-chip"
                    aria-pressed={filter === f}
                    onClick={() => setFilter(f)}
                    data-testid={`library-filter-${f}`}
                  >
                    {t(`library.filter.${f}`)}
                    <span className="library-chip__n">{counts[f]}</span>
                  </button>
                ))}
              </div>
            </div>

            {books && books.length === 0 ? (
              <div className="library-empty" data-testid="library-empty">
                <BookOpen size={32} aria-hidden="true" />
                <strong>{t("library.empty.title")}</strong>
                <span className="library-meta">{t("library.empty.text")}</span>
                <Button variant="primary" onClick={() => setAdding(true)}>
                  {t("library.add")}
                </Button>
              </div>
            ) : (
              <>
                {(continueViews.length > 0 || hasWork) &&
                  filter === "all" &&
                  !query && (
                    <div
                      className={`library-top ${hasWork ? "" : "library-top--solo"}`}
                    >
                      {continueViews.length > 0 && (
                        <section aria-labelledby="library-continue">
                          <h2
                            id="library-continue"
                            className="library-section__head"
                          >
                            {t("library.continue")}
                            <small>{t("library.continueHint")}</small>
                          </h2>
                          <div className="library-continue" ref={setContinueEl}>
                            {continueViews.map((v) => (
                              // Ganze Karte öffnet die Produktseite wie unter
                              // „Alle Titel“ (Patrick 03.11.); der Titel ist der
                              // Knopf und deckt die Karte ab, Abspielen liegt darüber.
                              <article
                                className="library-ccard"
                                key={v.book.id}
                                data-testid="library-continue-card"
                              >
                                {/* Abspielen liegt auf dem Cover: der Text hat
                                    so die ganze Breite (Patrick 04.11.). */}
                                <div className="library-ccard__media">
                                  <BookCover
                                    book={v.book}
                                    size="xs"
                                    className="library-ccard__cover"
                                  />
                                  {playButton(v, "library-ccard__play")}
                                </div>
                                <div className="library-ccard__body">
                                  <button
                                    type="button"
                                    className="library-ccard__title"
                                    onClick={() => setOpenId(v.book.id)}
                                    data-testid="library-continue-open"
                                  >
                                    {v.book.title}
                                  </button>
                                  {(v.book.authors ?? []).length > 0 && (
                                    <div className="library-ccard__author">
                                      {(v.book.authors ?? []).join(", ")}
                                    </div>
                                  )}
                                  <div className="library-ccard__foot">
                                    {(() => {
                                      const list = formatEntries(
                                        t,
                                        v,
                                        kurzbuecher[v.book.id],
                                      );
                                      const entry =
                                        list.find(
                                          (e) =>
                                            e.kind ===
                                            (v.resumeWith === "audio"
                                              ? "audiobook"
                                              : "text"),
                                        ) ?? list[0];
                                      return (
                                        <div className="library-ccard__progress">
                                          <ProgressBar
                                            fraction={
                                              entry?.fraction ?? v.fraction
                                            }
                                            label={t("library.progressLabel", {
                                              title: v.book.title,
                                            })}
                                          />
                                          {/* Alle Fassungen mit Wert, die
                                              fortzusetzende zuerst (Codex 04.11.). */}
                                          <span className="library-ccard__status">
                                            {entry
                                              ? [
                                                  entry,
                                                  ...list.filter(
                                                    (e) => e !== entry && !e.busy,
                                                  ),
                                                ].map((e) => (
                                                  <FormatValue
                                                    key={e.kind}
                                                    entry={e}
                                                  />
                                                ))
                                              : statusLine(v)}
                                          </span>
                                        </div>
                                      );
                                    })()}
                                  </div>
                                </div>
                              </article>
                            ))}
                          </div>
                        </section>
                      )}
                      {hasWork && (
                        <section
                          className="library-jobs"
                          aria-labelledby="library-jobs"
                        >
                          <h2
                            id="library-jobs"
                            className="library-section__head"
                          >
                            {t("library.working")}
                            <small>{t("library.workingHint")}</small>
                          </h2>
                          {jobList.map((job) => {
                            const book = books?.find(
                              (b) => b.id === job.bookId,
                            );
                            if (!book) return null;
                            return (
                              <div className="library-job" key={job.bookId}>
                                <BookCover book={book} size="xs" />
                                <div style={{ minWidth: 0 }}>
                                  <div
                                    className="library-title"
                                    style={{ WebkitLineClamp: 1 }}
                                  >
                                    {book.title}
                                  </div>
                                  <div className="library-meta">
                                    {jobLabel(t, job)}
                                  </div>
                                </div>
                              </div>
                            );
                          })}
                          {kiJobList.map((job) => {
                            const book = books?.find((b) => b.id === job.bookId);
                            if (!book) return null;
                            return (
                              <div
                                className="library-job library-job--ki"
                                key={job.id}
                                data-testid="library-kijob-row"
                              >
                                <BookCover book={book} size="xs" />
                                <div style={{ minWidth: 0 }}>
                                  <div
                                    className="library-title"
                                    style={{ WebkitLineClamp: 1 }}
                                  >
                                    {book.title}
                                  </div>
                                  <div className="library-meta">
                                    {jobTitle(t, job)} · {jobStatus(t, job)}
                                  </div>
                                  {job.status === "running" && (
                                    <div className="library-job__bar" aria-hidden="true">
                                      <span style={{ width: `${Math.round(jobFraction(job) * 100)}%` }} />
                                    </div>
                                  )}
                                </div>
                                <span className="library-touch">
                                  <IconButton
                                    icon={<X size={16} aria-hidden="true" />}
                                    label={t("library.ki.cancelJob", {
                                      what: jobTitle(t, job),
                                    })}
                                    onClick={() => cancelBookJob(job.id)}
                                    data-testid="library-kijob-cancel"
                                  />
                                </span>
                              </div>
                            );
                          })}
                        </section>
                      )}
                    </div>
                  )}
                {selecting && (
                  <div
                    className="library-selectbar"
                    role="toolbar"
                    aria-label={t("library.select.toolbar")}
                    data-testid="library-selectbar"
                  >
                    <span className="library-selectbar__text" aria-live="polite">
                      {t("library.select.count", { count: selected.length })}
                    </span>
                    <Button
                      onClick={() =>
                        setSelected(
                          selected.length === shown.length
                            ? []
                            : shown.map((v) => v.book.id),
                        )
                      }
                    >
                      {selected.length === shown.length && shown.length
                        ? t("library.select.none")
                        : t("library.select.all")}
                    </Button>
                    <Button
                      icon={<Layers size={16} aria-hidden="true" />}
                      disabled={selected.length < 2}
                      onClick={() => askManualMerge(selectedBooks)}
                      data-testid="library-select-merge"
                    >
                      {t("library.merge.action")}
                    </Button>
                    {selectedBooks.some((b) => hasTitleNoise(b.title)) && (
                      <Button
                        icon={<Eraser size={16} aria-hidden="true" />}
                        onClick={() =>
                          setCleaning(
                            selectedBooks.filter((b) => hasTitleNoise(b.title)),
                          )
                        }
                        data-testid="library-select-clean"
                      >
                        {t("library.clean.action")}
                      </Button>
                    )}
                    <Button
                      variant="danger-ghost"
                      icon={<Trash2 size={16} aria-hidden="true" />}
                      disabled={!selected.length}
                      onClick={() => setRemoving(selectedBooks)}
                      data-testid="library-select-remove"
                    >
                      {t("library.remove.action")}
                    </Button>
                    <Button onClick={endSelection} data-testid="library-select-end">
                      {t("library.select.end")}
                    </Button>
                  </div>
                )}
                {noisy.length > 0 && !query && !selecting && (
                  <div
                    className="library-mergebar"
                    role="status"
                    data-testid="library-clean-bar"
                  >
                    <Eraser size={18} aria-hidden="true" />
                    <span className="library-mergebar__text">
                      {t("library.clean.bar", { count: noisy.length })}
                    </span>
                    <Button
                      onClick={() => setCleaning(noisy)}
                      data-testid="library-clean-start"
                    >
                      {t("library.clean.action")}
                    </Button>
                  </div>
                )}
                {splitGroups.length > 0 && !query && !selecting && (
                  <div className="library-merges" data-testid="library-merges">
                    {splitGroups.map((g) => (
                      <div
                        className="library-mergebar"
                        key={g.key}
                        role="status"
                        data-testid="library-merge-bar"
                      >
                        <Layers size={18} aria-hidden="true" />
                        <span className="library-mergebar__text">
                          {t("library.merge.bar", {
                            count: g.books.length,
                            title: g.title,
                          })}
                        </span>
                        <Button
                          onClick={() => void askMerge(g)}
                          data-testid="library-merge-start"
                        >
                          {t("library.merge.action")}
                        </Button>
                        <span className="library-touch">
                          <IconButton
                            icon={<X size={16} aria-hidden="true" />}
                            label={t("library.merge.dismiss")}
                            onClick={() => dismissGroup(g.key)}
                          />
                        </span>
                      </div>
                    ))}
                  </div>
                )}
                <section aria-labelledby="library-all">
                  <h2 id="library-all" className="library-section__head">
                    {t("library.all")}
                    <small>{t("library.count", { count: shown.length })}</small>
                  </h2>
                  {shown.length ? (
                    <div className="library-grid">{shown.map(card)}</div>
                  ) : (
                    <p className="library-meta">{t("library.noMatch")}</p>
                  )}
                </section>
              </>
            )}
          </>
        )}
        {listening && playing && playingEdition && (
          <AudioBar
            key={`${listening.book.id}:${playingEdition.id}:${listening.follow}:${tracksOf(
              editionBook(playing, playingEdition).audio,
            )
              .map((t) => t.file)
              .join("|")}`}
            book={editionBook(playing, playingEdition)}
            edition={{
              id: playingEdition.id,
              audioKey: audioKeyOf(playing, playingEdition),
              full: playing,
            }}
            startFollow={listening.follow}
            startPlaying={listening.startPlaying}
            focusLanguage={listening.focusLanguage}
            loadingEditions={loadingEditions}
            onSwitch={(req) => void switchTo(req)}
            onAlign={() => {
              const other = playing.editions.find(
                (e) => e.id !== playingEdition.id,
              );
              if (other)
                setAligning({
                  bookId: playing.id,
                  pair: { a: playingEdition.id, b: other.id },
                });
            }}
            onSave={save}
            onHeight={setPlayerHeight}
            onClose={() => setListening(null)}
          />
        )}
        <PairsDialog
          open={pairsOpen}
          pairs={languagePairs.pairs}
          busy={pairsBusy}
          onConfirm={(chosen) => void joinChosen(chosen)}
          onCancel={() => setPairsOpen(false)}
        />
        <AlignDialog
          book={aligning ? (books?.find((b) => b.id === aligning.bookId) ?? null) : null}
          pair={aligning?.pair ?? null}
          onClose={() => setAligning(null)}
          onSaved={async () => {
            await reload();
            setAligning(null);
          }}
        />
        {offerDialog}
        <MergeDialog
          group={merging?.group ?? null}
          plan={merging?.plan ?? null}
          busy={!!merging?.busy}
          onConfirm={() => void doMerge()}
          onCancel={() => setMerging(null)}
          onMove={moveMergePart}
        />
        <RemoveDialog
          books={removing}
          onConfirm={(scope) => void doRemove(scope)}
          onCancel={() => setRemoving([])}
        />
        <CoverDialog
          book={coverFor}
          onClose={() => setCoverFor(null)}
          onChanged={async () => {
            await reload();
            toast(t("library.cover.done"));
          }}
        />
        <TrashDialog
          open={trashOpen}
          entries={trash}
          busy={trashBusy}
          notice={trashNotice}
          onRestore={(list) => void restoreFromTrash(list.map((e) => e.book))}
          onPurge={(list) => void purgeFromTrash(list.map((e) => e.book))}
          onClose={() => {
            setTrashOpen(false);
            setTrashNotice(null);
          }}
        />
        <KiCreateDialog
          book={kiFor?.book ?? null}
          tab={kiFor?.tab ?? "translation"}
          onTab={(tab) => setKiFor((k) => (k ? { ...k, tab } : k))}
          onClose={() => setKiFor(null)}
          onOpenDetail={() => {
            const id = kiFor?.book.id ?? null;
            setKiFor(null);
            setOpenId(id);
          }}
        />
        <ExportDialog book={exportFor} onClose={() => setExportFor(null)} />
        <CleanTitlesDialog
          books={cleaning}
          busy={cleanBusy}
          onConfirm={() => void doClean()}
          onCancel={() => setCleaning([])}
        />
        {dropDialog}
        <AddBookDialog
          open={adding}
          onOpenChange={setAdding}
          onImportRest={importPaths}
          onAdded={async (book, attached) => {
            await reload();
            const id = attached ? book.id : await offerFor(book);
            setOpenId(id);
          }}
        />
      </div>
    </PageShell>
  );
};

export default LibraryPage;
