import { useCallback, useEffect, useState } from "react";
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
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { Dialog } from "../../ui/Dialog";
import {
  ChevronDown,
  ChevronUp,
  FilePlus,
  Play,
  FolderOpen,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";

/**
 * Die Seitenliste links: welches Arbeitsblatt gerade offen ist, wie bei den
 * Unterhaltungen einer KI-App. Anlegen, umbenennen (Doppelklick), nach oben
 * und unten schieben, löschen — Löschen fragt nach, denn es nimmt den
 * Projektordner samt Dateien mit.
 */
export const PagesSidebar: React.FC<{
  pages: PageInfo[];
  activeId: string;
  collapsed: boolean;
  onToggle: () => void;
  onSelect: (id: string) => void;
  onChanged: () => void;
}> = ({ pages, activeId, collapsed, onToggle, onSelect, onChanged }) => {
  const { t } = useTranslation();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<PageInfo | null>(null);

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

  const move = async (id: string, delta: number) => {
    const ids = pages.map((p) => p.id);
    const from = ids.indexOf(id);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= ids.length) return;
    ids.splice(to, 0, ids.splice(from, 1)[0]);
    await commands.pagesReorder(ids);
    onChanged();
  };

  const remove = async (page: PageInfo) => {
    setDeleteTarget(null);
    await commands.pagesDelete(page.id);
    onChanged();
    if (page.id === activeId) {
      const rest = pages.filter((p) => p.id !== page.id);
      if (rest.length > 0) onSelect(rest[0].id);
    }
  };

  if (collapsed) {
    return (
      <div className="shrink-0 pt-1">
        <button
          type="button"
          onClick={onToggle}
          title={t("tts.pages.expand")}
          aria-label={t("tts.pages.expand")}
          className="p-1.5 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
        >
          <PanelLeftOpen width={18} height={18} />
        </button>
      </div>
    );
  }

  return (
    <div className="tts-workspace__pages w-52 shrink-0 space-y-1">
      <div className="flex items-center justify-between pb-1">
        <span className="text-xs font-semibold uppercase tracking-wide text-text/50">
          {t("tts.pages.title")}
        </span>
        <div className="flex items-center">
          <button
            type="button"
            onClick={create}
            title={t("tts.pages.create")}
            aria-label={t("tts.pages.create")}
            className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <Plus width={16} height={16} />
          </button>
          <button
            type="button"
            onClick={onToggle}
            title={t("tts.pages.collapse")}
            aria-label={t("tts.pages.collapse")}
            className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <PanelLeftClose width={16} height={16} />
          </button>
        </div>
      </div>

      {pages.map((page, index) => (
        <div
          key={page.id}
          className={`group flex items-center gap-1 rounded-md px-2 py-1.5 cursor-pointer transition-colors ${
            page.id === activeId
              ? "bg-logo-primary/15 text-text"
              : "text-text/70 hover:bg-mid-gray/15 hover:text-text"
          }`}
          onClick={() => onSelect(page.id)}
          onDoubleClick={() => {
            setEditingId(page.id);
            setEditTitle(page.title);
          }}
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
              <span className="flex-1 min-w-0 truncate text-sm">
                {page.title}
              </span>
              <span className="hidden group-hover:flex group-focus-within:flex items-center shrink-0">
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    void move(page.id, -1);
                  }}
                  disabled={index === 0}
                  title={t("tts.pages.moveUp")}
                  aria-label={t("tts.pages.moveUp")}
                  className="p-0.5 text-text/40 hover:text-text disabled:opacity-30 cursor-pointer"
                >
                  <ChevronUp width={14} height={14} />
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    void move(page.id, 1);
                  }}
                  disabled={index === pages.length - 1}
                  title={t("tts.pages.moveDown")}
                  aria-label={t("tts.pages.moveDown")}
                  className="p-0.5 text-text/40 hover:text-text disabled:opacity-30 cursor-pointer"
                >
                  <ChevronDown width={14} height={14} />
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
                  className="p-0.5 text-text/40 hover:text-text cursor-pointer"
                >
                  <Pencil width={13} height={13} />
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleteTarget(page);
                  }}
                  title={t("tts.pages.delete")}
                  aria-label={t("tts.pages.delete")}
                  className="p-0.5 text-red-400/60 hover:text-red-400 cursor-pointer"
                >
                  <Trash2 width={13} height={13} />
                </button>
              </span>
            </>
          )}
        </div>
      ))}

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
const isAudio = (name: string) =>
  AUDIO_EXTENSIONS.includes(name.split(".").pop()?.toLowerCase() ?? "");

export const FilesSidebar: React.FC<{
  pageId: string;
  collapsed: boolean;
  onToggle: () => void;
  /** Holt den Text einer erzeugten Aufnahme zurueck in den Editor. */
  onUseText?: (text: string) => void;
}> = ({ pageId, collapsed, onToggle, onUseText }) => {
  const { t } = useTranslation();
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
    void commands.pageDir(pageId).then((result) => {
      setDir(result.status === "ok" ? result.data : null);
    });
    setPlaying(null);
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
      <div className="shrink-0 pt-1">
        <button
          type="button"
          onClick={onToggle}
          title={t("tts.files.expand")}
          aria-label={t("tts.files.expand")}
          className="p-1.5 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
        >
          <PanelRightOpen width={18} height={18} />
        </button>
      </div>
    );
  }

  return (
    <div className="tts-workspace__files w-60 shrink-0 space-y-1">
      <div className="flex items-center justify-between pb-1">
        <span className="text-xs font-semibold uppercase tracking-wide text-text/50">
          {t("tts.files.title")}
        </span>
        <div className="flex items-center">
          <button
            type="button"
            onClick={addFile}
            title={t("tts.files.add")}
            aria-label={t("tts.files.add")}
            className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <FilePlus width={15} height={15} />
          </button>
          <button
            type="button"
            onClick={openFolder}
            title={t("tts.files.openFolder")}
            aria-label={t("tts.files.openFolder")}
            className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <FolderOpen width={15} height={15} />
          </button>
          <button
            type="button"
            onClick={() => void refresh()}
            title={t("tts.files.refresh")}
            aria-label={t("tts.files.refresh")}
            className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <RefreshCw width={14} height={14} />
          </button>
          <button
            type="button"
            onClick={onToggle}
            title={t("tts.files.collapse")}
            aria-label={t("tts.files.collapse")}
            className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 transition-colors cursor-pointer"
          >
            <PanelRightClose width={16} height={16} />
          </button>
        </div>
      </div>

      {error && <p className="text-xs text-red-400 break-words">{error}</p>}
      {files.length === 0 && (
        <p className="text-xs text-text/40">{t("tts.files.empty")}</p>
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
                <span className="flex-1 min-w-0 truncate text-sm">
                  {file.name}
                </span>
                <span className="text-[10px] text-text/35 shrink-0 group-hover:hidden group-focus-within:hidden">
                  {formatSize(file.size)}
                </span>
                {isAudio(file.name) && (
                  <button
                    type="button"
                    className="shrink-0 p-1 rounded hover:bg-mid-gray/25"
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
                            if (result.status === "ok") setNote(result.data);
                          });
                      }
                    }}
                  >
                    <Play width={12} height={12} />
                  </button>
                )}
                <span className="hidden group-hover:flex group-focus-within:flex items-center shrink-0">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      setEditingName(file.name);
                      setEditValue(file.name);
                    }}
                    title={t("tts.files.rename")}
                    aria-label={t("tts.files.rename")}
                    className="p-0.5 text-text/40 hover:text-text cursor-pointer"
                  >
                    <Pencil width={13} height={13} />
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      setDeleteTarget(file.name);
                    }}
                    title={t("tts.files.delete")}
                    aria-label={t("tts.files.delete")}
                    className="p-0.5 text-red-400/60 hover:text-red-400 cursor-pointer"
                  >
                    <Trash2 width={13} height={13} />
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
              <p className="text-[10px] text-text/45">
                {note.voice ?? t("tts.files.defaultVoice")} ·{" "}
                {new Date(note.created_ms).toLocaleString()}
              </p>
              {note.segments.length > 0 ? (
                /* Mit Zeitmarken laeuft der Text mit: der klingende Satz
                   steht hervorgehoben da, mit seinem Sprecher davor. Ohne
                   Zeitmarken (Aufnahmen aelterer Fassungen) bleibt der
                   Textanfang. */
                <ol className="space-y-0.5 max-h-40 overflow-y-auto">
                  {note.segments.map((segment, index) => {
                    const active =
                      atMs >= segment.start_ms && atMs < segment.end_ms;
                    return (
                      <li
                        key={`${segment.start_ms}-${index}`}
                        aria-current={active ? "true" : undefined}
                        className={
                          active
                            ? "text-[11px] text-text bg-logo-primary/25 rounded px-1"
                            : "text-[11px] text-text/45 px-1"
                        }
                      >
                        {segment.voice && (
                          <span className="text-text/40">
                            {segment.voice}:{" "}
                          </span>
                        )}
                        {segment.text}
                      </li>
                    );
                  })}
                </ol>
              ) : (
                <p className="text-[11px] text-text/60 line-clamp-3">
                  {note.text}
                </p>
              )}
              {onUseText && (
                <button
                  type="button"
                  className="text-[11px] underline text-text/70 hover:text-text"
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
