import { Fragment, useCallback, useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  commands,
  type AudioNote,
  type PageFile,
  type PageInfo,
} from "@/bindings";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { Input } from "../../ui/Input";
import { Dialog } from "../../ui/Dialog";
import { useContextMenu } from "../../ui/ContextMenu";
import { createPortal } from "react-dom";
import { useHelpDockSlot } from "../../shell/PageHeaderSlot";
import { splitFileNameTail } from "@/lib/utils/exportName";
import { HelpPanel } from "../../help/HelpPanel";
import { PageExportDialog, PageImportDialog } from "./pages/PagePackageDialogs";
import { GROUP_HEAD } from "./BookFolder";
import {
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  Columns2,
  FilePlus,
  Play,
  FolderOpen,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  PackageOpen,
  PackagePlus,
  Pencil,
  Plus,
  Replace,
  RefreshCw,
  Rows2,
  Square,
  Trash2,
  X,
} from "lucide-react";

/**
 * „vor 3 Min.", „gestern", „vor 2 Wochen" — in der Sprache der Oberflaeche,
 * ohne eigene Uebersetzungsschluessel: das kann der Browser.
 */
export const relativeTime = (ms: number, locale: string): string => {
  const diff = ms - Date.now();
  const abs = Math.abs(diff);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 365 * 86_400_000],
    ["month", 30 * 86_400_000],
    ["week", 7 * 86_400_000],
    ["day", 86_400_000],
    ["hour", 3_600_000],
    ["minute", 60_000],
  ];
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  for (const [unit, size] of units) {
    if (abs >= size) return rtf.format(Math.round(diff / size), unit);
  }
  return rtf.format(0, "second");
};

/**
 * Zeilenaktionen der Leisten: 16-px-Symbol in `p-1` ergibt 24x24 px Klickflaeche
 * (WCAG 2.5.8) -- vorher 17x17 mit 13-px-Symbol. Feste 4 px statt `p-1`:
 * bei 15-px-Wurzelschrift waeren das 3,75 px und damit 23,5 px Flaeche. Eingeblendet werden sie
 * weiterhin erst beim Ueberfahren der Zeile.
 */
const ROW_ACTION =
  "p-[4px] rounded text-text/60 hover:text-text hover:bg-mid-gray/25 disabled:opacity-30 cursor-pointer transition-colors";
const ROW_ACTION_DANGER =
  "p-[4px] rounded text-red-400/70 hover:text-red-400 hover:bg-mid-gray/25 cursor-pointer transition-colors";
/** Dateizeilen: die Aktionen sind immer sichtbar; Loeschen bleibt grau und
 *  wird erst bei Hover/Fokus rot, damit es nicht die ganze Liste faerbt. */
const ROW_ACTION_QUIET_DANGER =
  "p-[4px] rounded text-text/60 hover:text-red-400 focus-visible:text-red-400 hover:bg-mid-gray/25 cursor-pointer transition-colors";

/**
 * Die Seitenliste links: welches Arbeitsblatt gerade offen ist, wie bei den
 * Unterhaltungen einer KI-App. Anlegen, umbenennen (Doppelklick), nach oben
 * und unten schieben, löschen — Löschen fragt nach, denn es nimmt den
 * Projektordner samt Dateien mit.
 */
/** Eingeklappte Spalte: links wie rechts derselbe schmale Streifen mit nur
 *  einem Symbol oben, Erklärung im Tooltip (Patrick 01.11.). Der Streifen
 *  nimmt die halbe Spaltenlücke mit, damit er schmal bleibt. */
export const ColumnStrip = ({
  side,
  label,
  onClick,
  testId,
}: {
  side: "left" | "right";
  label: string;
  onClick: () => void;
  testId?: string;
}) => (
  <div
    className={`tts-strip shrink-0 pt-1 ${side === "left" ? "-me-2" : "-ms-2"}`}
    data-testid={testId ? `${testId}-strip` : undefined}
  >
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      data-testid={testId}
      className="flex h-7 w-6 cursor-pointer items-center justify-center rounded-md text-text/60 transition-colors hover:bg-mid-gray/20 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
    >
      {side === "left" ? (
        <PanelLeftOpen width={16} height={16} aria-hidden="true" />
      ) : (
        <PanelRightOpen width={16} height={16} aria-hidden="true" />
      )}
    </button>
  </div>
);

export const PagesSidebar: React.FC<{
  pages: PageInfo[];
  activeId: string;
  collapsed: boolean;
  onToggle: () => void;
  onSelect: (id: string) => void;
  onChanged: () => void;
  /** Eintrag "Sprecher auf allen Seiten ersetzen" im Rechtsklick-Menue. */
  onReplaceSpeaker?: () => void;
  /** Breite in Pixeln (Ziehgriff); ohne Angabe gilt die Standardbreite. */
  width?: number;
  /** Ordnerleiste über der Liste (lva#365): Alle, Buch-Ordner, Texte. */
  folders?: ReactNode;
  /** Nur diese Seiten zeigen (Auswahl der Ordnerleiste); ohne Angabe alle. */
  visibleIds?: string[];
  /** Sprache je Seite, wenn sie Fassung eines Buchs ist („de“, „en“ …). */
  langOf?: Map<string, string>;
  /** Inhalte der offenen Seite (Original, Übersetzung, KI-Dokumente) unter ihrer Zeile (lva#365 E2). */
  activeSections?: ReactNode;
  /** Inhaltsverzeichnis, nur wenn die rechte Spalte nicht sichtbar ist (lva#365). */
  contents?: ReactNode;
  /**
   * Buch-Ordner gewählt (lva#365 P1b): Kopf über den Fassungszeilen, nicht
   * auffindbare Fassungen darunter, danach die Gruppen Hörbuch und KI-Dokumente.
   */
  bookFolder?: { head: ReactNode; missing: ReactNode; after: ReactNode };
  /** Beschriftung „Sprache · Hauptfassung“ je Seite (nur im Buch-Ordner). */
  editionNote?: Map<string, string>;
  /** Text für einen gewählten Ordner ohne Seiten; leer = kein Leerzustand. */
  emptyLabel?: string;
  /** Schmal (≤ 620 px): Ebene Ordner, Liste oder Reader. */
  mobileLevel?: "folders" | "list" | "reader";
  /** Schmal: Zurück von der Liste zu den Ordnern. */
  onBack?: () => void;
  /** Eine Seite wurde bewusst geöffnet (Zeile, Weiterlesen, Hörbuch, KI-Dokument). */
  onOpened?: () => void;
}> = ({
  pages,
  activeId,
  collapsed,
  onToggle,
  onSelect,
  onChanged,
  onReplaceSpeaker,
  width,
  folders,
  visibleIds,
  langOf,
  activeSections,
  contents,
  bookFolder,
  editionNote,
  emptyLabel,
  mobileLevel = "reader",
  onBack,
  onOpened,
}) => {
  const { t, i18n } = useTranslation();
  const shown = visibleIds
    ? pages.filter((p) => visibleIds.includes(p.id))
    : pages;
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<PageInfo | null>(null);
  const [exportTarget, setExportTarget] = useState<PageInfo | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const rowMenu = useContextMenu(t("tts.pages.title"));

  const create = async () => {
    const result = await commands.pagesCreate("");
    if (result.status === "ok") {
      onChanged();
      onSelect(result.data.id);
    }
  };

  const commitRename = async () => {
    if (editingId && editTitle.trim()) {
      await commands.pagesRename(editingId, editTitle.trim());
      onChanged();
    }
    setEditingId(null);
  };

  // Verschoben wird innerhalb der sichtbaren Liste: die Seite tauscht den Platz
  // mit ihrer sichtbaren Nachbarin, die übrigen Seiten bleiben, wo sie sind.
  const move = async (id: string, delta: number) => {
    const visible = shown.map((p) => p.id);
    const at = visible.indexOf(id);
    const other = visible[at + delta];
    if (at < 0 || other === undefined) return;
    const ids = pages.map((p) => p.id);
    const a = ids.indexOf(id);
    const b = ids.indexOf(other);
    [ids[a], ids[b]] = [ids[b], ids[a]];
    await commands.pagesReorder(ids);
    onChanged();
  };

  const remove = async (page: PageInfo) => {
    setDeleteTarget(null);
    await commands.pagesDelete(page.id);
    onChanged();
    if (page.id === activeId) {
      // Nachfolger aus der sichtbaren Liste (Nachbar), damit der Ordner bleibt.
      const at = shown.findIndex((p) => p.id === page.id);
      const visibleRest = shown.filter((p) => p.id !== page.id);
      const next =
        visibleRest[Math.min(Math.max(at, 0), visibleRest.length - 1)] ??
        pages.find((p) => p.id !== page.id);
      if (next) onSelect(next.id);
    }
  };

  if (collapsed) {
    return (
      <ColumnStrip
        side="left"
        label={t("tts.pages.expand")}
        onClick={onToggle}
        testId="pages-expand"
      />
    );
  }

  return (
    <div
      data-testid="tts-pages"
      data-mlevel={mobileLevel}
      className="tts-workspace__pages shrink-0 space-y-1 min-h-0 overflow-y-auto"
      style={width ? ({ "--tts-w": `${width}px` } as CSSProperties) : undefined}
    >
      <div className="flex items-center justify-between pb-1">
        <span className="text-xs font-semibold text-text/60">
          {t("tts.pages.title")}
        </span>
        <div className="lv-touch-icons flex items-center">
          <IconButton
            onClick={create}
            label={t("tts.pages.create")}
            icon={<Plus width={16} height={16} />}
          />
          <IconButton
            onClick={() => setImportOpen(true)}
            label={t("tts.pages.package.importTitle")}
            data-testid="page-import-open"
            icon={<PackageOpen width={16} height={16} />}
          />
          <IconButton
            onClick={onToggle}
            label={t("tts.pages.collapse")}
            icon={<PanelLeftClose width={16} height={16} />}
          />
        </div>
      </div>

      {onBack && (
        <button
          type="button"
          className="tts-mback tts-mback--list"
          onClick={onBack}
          title={t("tts.folders.backToFolders")}
          data-testid="mback-list"
        >
          <ArrowLeft width={18} height={18} aria-hidden="true" />
          {/* Name = „Zurück“ + sichtbares „Ordner“ (WCAG 2.5.3). */}
          <span className="sr-only">{t("tts.folders.back")} </span>
          <span>{t("tts.folders.label")}</span>
        </button>
      )}

      {folders}

      {bookFolder?.head}
      {bookFolder && (
        <p className={`${GROUP_HEAD} tts-pages__list`}>
          {t("tts.folders.groupEditions")}
        </p>
      )}

      {shown.map((page, index) => (
        <Fragment key={page.id}>
        <div
          className={`tts-pages__list group relative flex items-center gap-1 rounded-md px-2 py-1.5 cursor-pointer transition-colors ${
            page.id === activeId
              ? "bg-logo-primary/15 text-text"
              : "text-text/70 hover:bg-mid-gray/15 hover:text-text"
          }`}
          onClick={() => {
            onSelect(page.id);
            onOpened?.();
          }}
          onDoubleClick={() => {
            setEditingId(page.id);
            setEditTitle(page.title);
          }}
          onContextMenu={(event) =>
            rowMenu.open(event, [
              {
                id: "rename",
                label: t("tts.pages.rename"),
                icon: Pencil,
                testId: "page-menu-rename",
                onSelect: () => {
                  setEditingId(page.id);
                  setEditTitle(page.title);
                },
              },
              {
                id: "up",
                label: t("tts.pages.moveUp"),
                icon: ChevronUp,
                disabled: index === 0,
                onSelect: () => void move(page.id, -1),
              },
              {
                id: "down",
                label: t("tts.pages.moveDown"),
                icon: ChevronDown,
                disabled: index === shown.length - 1,
                onSelect: () => void move(page.id, 1),
              },
              {
                id: "export",
                label: t("tts.pages.package.exportTitle"),
                icon: PackagePlus,
                onSelect: () => setExportTarget(page),
              },
              ...(onReplaceSpeaker
                ? [
                    {
                      id: "replace",
                      label: t("tts.replaceSpeaker.voiceMenu"),
                      icon: Replace,
                      testId: "page-menu-replace-speaker",
                      separatorBefore: true,
                      onSelect: onReplaceSpeaker,
                    },
                  ]
                : []),
              {
                id: "delete",
                label: t("tts.pages.delete"),
                icon: Trash2,
                danger: true,
                separatorBefore: true,
                testId: "page-menu-delete",
                onSelect: () => setDeleteTarget(page),
              },
            ])
          }
        >
          {editingId === page.id ? (
            <Input
              type="text"
              variant="compact"
              value={editTitle}
              autoFocus
              onChange={(e) => setEditTitle(e.target.value)}
              onBlur={() => void commitRename()}
              onKeyDown={(e) => {
                if (e.key === "Enter") void commitRename();
                if (e.key === "Escape") setEditingId(null);
              }}
              onClick={(e) => e.stopPropagation()}
              className="w-full"
            />
          ) : (
            <>
              <span className="flex-1 min-w-0">
                <span className="flex items-center gap-1.5">
                  <span className="min-w-0 truncate text-sm">{page.title}</span>
                  {langOf?.get(page.id) && (
                    <span className="shrink-0 rounded bg-mid-gray/15 px-1 text-[11px] uppercase text-text/70">
                      {langOf.get(page.id)}
                    </span>
                  )}
                </span>
                {editionNote?.get(page.id) && (
                  <span className="block text-xs text-text/60">
                    {editionNote.get(page.id)}
                  </span>
                )}
                {/* Verlauf statt blosser Titel: was drinsteht und wann es
                    zuletzt angefasst wurde, ohne die Seite zu oeffnen. */}
                <span className="block truncate text-xs text-text/60">
                  {page.preview || t("tts.pages.emptyPreview")}
                </span>
                {(page.modified_ms ?? 0) > 0 && (
                  <span className="block text-xs text-text/60">
                    {relativeTime(page.modified_ms ?? 0, i18n.language)}
                  </span>
                )}
              </span>
              {/* Schwebt über der Zeile statt Platz zu belegen: fünf Aktionen à
                    24 px würden in der schmalen Leiste Titel und Vorschau auf
                    wenige Zeichen zusammendrücken. */}
              <span className="hidden group-hover:flex group-focus-within:flex items-center absolute end-1 top-1 rounded-md border border-mid-gray/20 bg-background shadow-sm">
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    void move(page.id, -1);
                  }}
                  disabled={index === 0}
                  title={t("tts.pages.moveUp")}
                  aria-label={t("tts.pages.moveUp")}
                  className={ROW_ACTION}
                >
                  <ChevronUp width={16} height={16} />
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    void move(page.id, 1);
                  }}
                  disabled={index === shown.length - 1}
                  title={t("tts.pages.moveDown")}
                  aria-label={t("tts.pages.moveDown")}
                  className={ROW_ACTION}
                >
                  <ChevronDown width={16} height={16} />
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditingId(page.id);
                    setEditTitle(page.title);
                  }}
                  title={t("tts.pages.rename")}
                  aria-label={t("tts.pages.rename")}
                  className={ROW_ACTION}
                >
                  <Pencil width={16} height={16} />
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    setExportTarget(page);
                  }}
                  title={t("tts.pages.package.exportTitle")}
                  aria-label={t("tts.pages.package.exportTitle")}
                  data-testid="page-export-open"
                  className={ROW_ACTION}
                >
                  <PackagePlus width={16} height={16} />
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleteTarget(page);
                  }}
                  title={t("tts.pages.delete")}
                  aria-label={t("tts.pages.delete")}
                  className={ROW_ACTION_DANGER}
                >
                  <Trash2 width={16} height={16} />
                </button>
              </span>
            </>
          )}
        </div>
        {page.id === activeId && activeSections && (
          <div className="ms-3 border-s border-mid-gray/20 ps-1" data-testid="page-sections">
            {activeSections}
          </div>
        )}
        </Fragment>
      ))}

      {bookFolder?.missing}
      {bookFolder?.after}

      {!bookFolder && shown.length === 0 && emptyLabel && (
        <div
          className="tts-pages__list px-2 py-3 space-y-2"
          data-testid="pages-empty"
        >
          <p className="text-sm text-text/80">{emptyLabel}</p>
          <Button
            variant="secondary"
            className="reader-folders__btn"
            icon={<Plus width={16} height={16} aria-hidden="true" />}
            onClick={() => {
              void create().then(() => onOpened?.());
            }}
          >
            {t("tts.folders.newPage")}
          </Button>
        </div>
      )}

      {contents && <div className="tts-pages__list">{contents}</div>}

      {rowMenu.menu}
      <PageExportDialog
        page={exportTarget}
        onClose={() => setExportTarget(null)}
      />
      <PageImportDialog
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={(page) => {
          onChanged();
          onSelect(page.id);
        }}
      />
      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setDeleteTarget(null);
        }}
        title={t("tts.pages.deleteConfirmTitle")}
        closeLabel={t("tts.stopConfirmCancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              {t("tts.stopConfirmCancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => deleteTarget && void remove(deleteTarget)}
            >
              {t("tts.pages.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("tts.pages.deleteConfirm", { title: deleteTarget?.title ?? "" })}
        </p>
      </Dialog>
    </div>
  );
};

/** Lesbare Dateigröße — Dateilisten ohne Größe zwingen zum Raten. */
const formatSize = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

/**
 * Die Dateileiste rechts: der Projektordner der offenen Seite. Erzeugte
 * Audiodateien landen hier von selbst (der Speichern-Dialog schlägt diesen
 * Ordner vor); Dokumente kommen per „Hinzufügen" als Kopie dazu. Öffnen mit
 * der Standardanwendung, Umbenennen per Doppelklick, Löschen mit Rückfrage.
 */
/// Welche Dateien die Leiste selbst abspielen kann. Alles andere oeffnet
/// weiterhin das Programm des Systems.
const AUDIO_EXTENSIONS = ["wav", "mp3", "opus", "flac", "ogg", "m4a"];

export type RightTab = "files" | "help";
export const isRightTab = (value: string): value is RightTab =>
  value === "files" || value === "help";
const isAudio = (name: string) =>
  AUDIO_EXTENSIONS.includes(name.split(".").pop()?.toLowerCase() ?? "");

export const FilesSidebar: React.FC<{
  pageId: string;
  collapsed: boolean;
  onToggle: () => void;
  /** Holt den Text einer erzeugten Aufnahme zurueck in den Editor. */
  onUseText?: (text: string) => void;
  /** Breite in Pixeln (Ziehgriff) im Nebeneinander-Layout. */
  width?: number;
  /** Gestapelt unter der Bedienung: volle Spaltenbreite, füllt die Resthöhe. */
  stacked?: boolean;
  /** Umschalter Gestapelt/Nebeneinander im Kopf; ohne ihn kein Knopf. */
  onLayoutToggle?: () => void;
  /** Inhaltsverzeichnis der offenen Seite (Leseansicht, lva#365: rechts statt links). */
  contents?: ReactNode;
}> = ({
  pageId,
  collapsed,
  onToggle,
  onUseText,
  width,
  stacked = false,
  onLayoutToggle,
  contents,
}) => {
  const { t, i18n } = useTranslation();
  const [files, setFiles] = useState<PageFile[]>([]);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Welche Audiodatei gerade angehoert wird, und wo der Ordner liegt. Ohne
  // beides muesste man eine erzeugte Aufnahme erst im Dateimanager oeffnen,
  // um zu hoeren, was man erzeugt hat.
  const [playing, setPlaying] = useState<string | null>(null);
  const [dir, setDir] = useState<string | null>(null);
  // Herkunft der aufgeklappten Aufnahme: Text, Stimme, Zeitpunkt. Erst beim
  // Aufklappen geholt — fuer eine Liste mit dreissig Aufnahmen waeren
  // dreissig Dateizugriffe beim Oeffnen der Leiste zu viel.
  const [note, setNote] = useState<AudioNote | null>(null);
  // Sekunde, an der die Wiedergabe gerade steht — daraus ergibt sich, welcher
  // Satz klingt. Nur beim Abspielen gefuehrt.
  const [atMs, setAtMs] = useState(0);

  useEffect(() => {
    if (!pageId) return;
    let alive = true;
    const load = () =>
      void commands.pageDir(pageId).then((result) => {
        if (alive) setDir(result.status === "ok" ? result.data : null);
      });
    load();
    setPlaying(null);
    // Nach dem Umzug in Buch-Ordner (lva#365) liegt die Seite woanders: den
    // gemerkten Pfad neu holen, sonst zeigt die Wiedergabe ins Leere.
    const unlisten = listen("library-layout-changed", load);
    return () => {
      alive = false;
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [pageId]);

  const refresh = useCallback(async () => {
    if (!pageId) return;
    const result = await commands.pageFiles(pageId);
    if (result.status === "ok") setFiles(result.data);
  }, [pageId]);

  useEffect(() => {
    void refresh();
    // Exporte melden sich hierüber, sobald ihre Datei fertig ist.
    const handler = () => void refresh();
    window.addEventListener("lv-files-changed", handler);
    return () => window.removeEventListener("lv-files-changed", handler);
  }, [refresh]);

  const addFile = async () => {
    const picked = await open({ multiple: false });
    if (typeof picked !== "string") return;
    setError(null);
    const result = await commands.pageFileAdd(pageId, picked);
    if (result.status === "error") setError(result.error);
    void refresh();
  };

  const openFolder = async () => {
    const dir = await commands.pageDir(pageId);
    if (dir.status === "ok") {
      // Den Ordner selbst zeigen — revealItemInDir erwartet einen Eintrag
      // darin, deshalb der Umweg über die erste Datei, sonst der Ordner.
      await revealItemInDir(
        files.length > 0 ? `${dir.data}\\${files[0].name}` : dir.data,
      );
    }
  };

  const commitRename = async () => {
    if (editingName && editValue.trim() && editValue !== editingName) {
      const result = await commands.pageFileRename(
        pageId,
        editingName,
        editValue.trim(),
      );
      if (result.status === "error") setError(result.error);
      void refresh();
    }
    setEditingName(null);
  };

  const remove = async (name: string) => {
    setDeleteTarget(null);
    const result = await commands.pageFileDelete(pageId, name);
    if (result.status === "error") setError(result.error);
    void refresh();
  };

  if (collapsed) {
    return (
      // Rechtsbuendig: gestapelt ist das eine Flex-Spalte, in der der Knopf
      // sonst links unter der Bedienung klebte; nebeneinander bleibt er am
      // rechten Rand. Gleiche Flaeche (p-1) wie die Knoepfe im Leistenkopf.
      <div className="shrink-0 pt-1 flex justify-end">
        <IconButton
          onClick={onToggle}
          label={t("tts.files.expand")}
          data-testid="files-expand"
          icon={<PanelRightOpen width={16} height={16} />}
        />
      </div>
    );
  }

  return (
    <div
      data-testid="tts-files"
      className={`tts-workspace__files space-y-1 min-h-0 overflow-y-auto ${
        stacked ? "" : "shrink-0"
      }`}
      style={
        !stacked && width
          ? ({ "--tts-w": `${width}px` } as CSSProperties)
          : undefined
      }
    >
      <div className="flex items-center justify-between pb-1">
        <span className="text-xs font-semibold text-text/60">
          {t("tts.files.title")}
        </span>
        <div className="lv-touch-icons flex items-center">
          {(
            <>
              <IconButton
                onClick={addFile}
                label={t("tts.files.add")}
                icon={<FilePlus width={16} height={16} />}
              />
              <IconButton
                onClick={openFolder}
                label={t("tts.files.openFolder")}
                icon={<FolderOpen width={16} height={16} />}
              />
              <IconButton
                onClick={() => void refresh()}
                label={t("tts.files.refresh")}
                icon={<RefreshCw width={16} height={16} />}
              />
            </>
          )}
          {onLayoutToggle && (
            <IconButton
              onClick={onLayoutToggle}
              data-testid="layout-toggle"
              aria-pressed={!stacked}
              label={t("tts.layout.sideBySide")}
              icon={
                stacked ? (
                  <Rows2 width={16} height={16} />
                ) : (
                  <Columns2 width={16} height={16} />
                )
              }
            />
          )}
          <IconButton
            onClick={onToggle}
            label={t("tts.files.collapse")}
            icon={<PanelRightClose width={16} height={16} />}
          />
        </div>
      </div>

      {contents && (
        <div className="border-b border-mid-gray/20 pb-2 mb-1" data-testid="reader-contents-right">
          {contents}
        </div>
      )}

      {(
        <>
          {error && <p className="text-xs text-red-400 break-words">{error}</p>}
          {files.length === 0 && (
            <p className="text-xs text-text/60">{t("tts.files.empty")}</p>
          )}

          {files.map((file) => (
            <div key={file.name}>
              <div
                className="group flex items-center gap-1 rounded-md px-2 py-1.5 text-text/70 hover:bg-mid-gray/15 hover:text-text cursor-pointer transition-colors"
                onClick={() => void commands.pageFileOpen(pageId, file.name)}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  setEditingName(file.name);
                  setEditValue(file.name);
                }}
                title={t("tts.files.openHint")}
              >
                {editingName === file.name ? (
                  <Input
                    type="text"
                    variant="compact"
                    value={editValue}
                    autoFocus
                    onChange={(e) => setEditValue(e.target.value)}
                    onBlur={() => void commitRename()}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void commitRename();
                      if (e.key === "Escape") setEditingName(null);
                    }}
                    onClick={(e) => e.stopPropagation()}
                    className="w-full"
                  />
                ) : (
                  <>
                    {/* Mittig kuerzen: der Anfang schrumpft mit "...", der
                        Zeitstempel am Ende bleibt lesbar -- er unterscheidet
                        die Fassungen. Nur CSS, keine Messung. */}
                    <span className="flex flex-1 min-w-0 flex-col">
                      <span className="flex text-sm" title={file.name}>
                        <span className="truncate">
                          {splitFileNameTail(file.name).head}
                        </span>
                        <span className="shrink-0 whitespace-pre">
                          {splitFileNameTail(file.name).tail}
                        </span>
                      </span>
                      {/* Groesse als eigene Metazeile: frueher tauschte sie beim
                          Hover den Platz mit den Aktionen, und der Papierkorb
                          landete dort, wo eben noch "Anhoeren" stand. */}
                      <span className="text-xs text-text/60">
                        {formatSize(file.size)}
                      </span>
                    </span>
                    {isAudio(file.name) ? (
                      <button
                        type="button"
                        className={`shrink-0 p-[4px] rounded cursor-pointer transition-colors ${
                          playing === file.name
                            ? "bg-logo-primary/15 text-text"
                            : "text-text/60 hover:text-text hover:bg-mid-gray/25"
                        }`}
                        title={t("tts.files.listen")}
                        aria-label={t("tts.files.listen")}
                        aria-pressed={playing === file.name}
                        onClick={(e) => {
                          e.stopPropagation();
                          const next = playing === file.name ? null : file.name;
                          setPlaying(next);
                          setNote(null);
                          setAtMs(0);
                          if (next) {
                            void commands
                              .pageAudioNote(pageId, next)
                              .then((result) => {
                                if (result.status === "ok")
                                  setNote(result.data);
                              });
                          }
                        }}
                      >
                        {/* Symbol folgt dem Zustand: solange der Player offen
                            ist, steht dort Stopp -- nicht nur aria-pressed. */}
                        {playing === file.name ? (
                          <Square width={16} height={16} fill="currentColor" />
                        ) : (
                          <Play width={16} height={16} />
                        )}
                      </button>
                    ) : (
                      // Platz des Anhoeren-Knopfs bleibt frei, damit
                      // Umbenennen und Loeschen in jeder Zeile gleich stehen.
                      <span className="w-6 shrink-0" aria-hidden="true" />
                    )}
                    <span className="flex items-center shrink-0">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setEditingName(file.name);
                          setEditValue(file.name);
                        }}
                        title={t("tts.files.rename")}
                        aria-label={t("tts.files.rename")}
                        className={ROW_ACTION}
                      >
                        <Pencil width={16} height={16} />
                      </button>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleteTarget(file.name);
                        }}
                        title={t("tts.files.delete")}
                        aria-label={t("tts.files.delete")}
                        className={ROW_ACTION_QUIET_DANGER}
                      >
                        <Trash2 width={16} height={16} />
                      </button>
                    </span>
                  </>
                )}
              </div>
              {playing === file.name && dir && (
                /* Bewusst die Steuerung des Systems statt des hauseigenen
               AudioPlayer: der ist fuer breite Flaechen gebaut und
               bricht in dieser schmalen Spalte in eine Saeule
               auseinander. Eine funktionierende Leiste schlaegt eine
               huebsche, die zerfaellt. */
                <audio
                  controls
                  preload="metadata"
                  onTimeUpdate={(event) =>
                    setAtMs(event.currentTarget.currentTime * 1000)
                  }
                  className="w-full mt-1 mb-2"
                  src={convertFileSrc(`${dir}\\${file.name}`, "asset")}
                />
              )}
              {playing === file.name && note && (
                <div className="mb-2 space-y-1">
                  <p className="text-xs text-text/60">
                    {note.voice ?? t("tts.files.defaultVoice")} ·{" "}
                    {new Date(note.created_ms).toLocaleString(i18n.language)}
                  </p>
                  {(note.segments ?? []).length > 0 ? (
                    /* Mit Zeitmarken laeuft der Text mit: der klingende Satz
                   steht hervorgehoben da, mit seinem Sprecher davor. Ohne
                   Zeitmarken (Aufnahmen aelterer Fassungen) bleibt der
                   Textanfang. */
                    <ol className="space-y-0.5 max-h-40 overflow-y-auto">
                      {(note.segments ?? []).map((segment, index) => {
                        const active =
                          atMs >= segment.start_ms && atMs < segment.end_ms;
                        return (
                          <li
                            key={`${segment.start_ms}-${index}`}
                            aria-current={active ? "true" : undefined}
                            className={
                              active
                                ? "text-xs text-text bg-logo-primary/25 rounded px-1"
                                : "text-xs text-text/60 px-1"
                            }
                          >
                            {segment.voice && (
                              <span className="text-text/60">
                                {segment.voice}:{" "}
                              </span>
                            )}
                            {segment.text}
                          </li>
                        );
                      })}
                    </ol>
                  ) : (
                    <p className="text-xs text-text/60 line-clamp-3">
                      {note.text}
                    </p>
                  )}
                  {onUseText && (
                    <button
                      type="button"
                      className="text-xs underline text-text/70 hover:text-text"
                      onClick={(e) => {
                        e.stopPropagation();
                        onUseText(note.text);
                      }}
                    >
                      {t("tts.files.useText")}
                    </button>
                  )}
                </div>
              )}
            </div>
          ))}
        </>
      )}
      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setDeleteTarget(null);
        }}
        title={t("tts.files.deleteConfirmTitle")}
        closeLabel={t("tts.stopConfirmCancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              {t("tts.stopConfirmCancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => deleteTarget && void remove(deleteTarget)}
            >
              {t("tts.files.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("tts.files.deleteConfirm", { name: deleteTarget ?? "" })}
        </p>
      </Dialog>
    </div>
  );
};

/**
 * Hilfe zu Vorlesen: dockt wie die Seitenhilfe aller Seiten (PageShell) als
 * eigene Spalte am rechten Fensterrand an. Die Dateiliste bleibt, wo sie ist.
 * Ohne Dock (Tests) steht sie nicht im Inhalt, sondern gar nicht.
 */
export const TtsHelpDock: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const { t } = useTranslation();
  const dock = useHelpDockSlot();
  if (!dock) return null;
  return createPortal(
    <aside
      className="lv-helpdock__panel space-y-2"
      aria-label={t("help.title")}
    >
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-text/50">
          {t("help.title")}
        </span>
        <IconButton
          onClick={onClose}
          label={t("help.close")}
          icon={<X width={14} height={14} />}
        />
      </div>
      <HelpPanel section="vorlesen" />
    </aside>,
    dock,
  );
};
