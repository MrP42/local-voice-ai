import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  BookOpen,
  CloudDownload,
  FilePlus,
  FileText,
  Headphones,
  NotebookPen,
  Paperclip,
  Sparkles,
  Trash2,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  commands,
  type BookFolderFile,
  type Edition,
  type KiDocument,
  type LibraryBook,
} from "@/bindings";
import { BookCover } from "../../library/BookCover";
import { Button } from "../../ui/Button";
import { formatDuration } from "@/lib/library/books";
import { editionsOf, langName, primaryEdition } from "@/lib/library/editions";
import { tracksOf } from "@/lib/library/names";
import { formatBytes } from "@/lib/library/remove";

/**
 * Buch-Ordner im Vorlesen (lva#365 P1b): Kopf mit Cover und „Weiterlesen“,
 * dazu die Gruppen Fassungen, Hörbuch und KI-Dokumente. Die Fassungszeilen
 * selbst zeichnet die Seitenliste (Aktionen, Ansichten der offenen Seite);
 * hier stehen die Teile drumherum.
 */

/** Seiten-ID der Fassung, falls sie eine hat. */
const pageOf = (e: Edition): string | null => e.text?.page_id ?? null;

/** Sprache der Fassung zur Anzeige; ohne Angabe ein Platzhalter. */
export const editionLang = (
  e: Pick<Edition, "lang">,
  fallback: string,
): string => langName(e.lang) || fallback;

/** Seiten-ID → Beschriftung „Sprache · Hauptfassung“ für die Fassungszeilen. */
export function editionNotes(
  book: LibraryBook,
  mainLabel: string,
  unknown: string,
): Map<string, string> {
  const main = primaryEdition(book);
  const out = new Map<string, string>();
  for (const e of editionsOf(book)) {
    const page = pageOf(e);
    if (!page || out.has(page)) continue;
    const lang = editionLang(e, unknown);
    out.set(page, e.id === main?.id ? `${lang} · ${mainLabel}` : lang);
  }
  return out;
}

/** KI-Dokumente je Fassungsseite; lädt neu, wenn sich Seiten oder `refresh` ändern. */
export function useBookKiDocs(
  pageIds: string[],
  refresh: unknown,
): Map<string, KiDocument[]> {
  const key = pageIds.join("\n");
  const [docs, setDocs] = useState<Map<string, KiDocument[]>>(new Map());
  // Ein KI-Dokument einer anderen Fassung (Hintergrundlauf, Bibliothek) meldet
  // sich über `lv-kidocs-changed`: dann die Gruppe neu laden.
  const [changed, setChanged] = useState(0);
  useEffect(() => {
    const on = () => setChanged((n) => n + 1);
    window.addEventListener("lv-kidocs-changed", on);
    return () => window.removeEventListener("lv-kidocs-changed", on);
  }, []);
  useEffect(() => {
    let alive = true;
    const ids = key ? key.split("\n") : [];
    void Promise.all(
      ids.map(async (id) => {
        const r = await commands.bookAiDocuments(id).catch(() => null);
        const list =
          r && r.status === "ok" && Array.isArray(r.data) ? r.data : [];
        return [id, list] as const;
      }),
    ).then((entries) => {
      if (alive) setDocs(new Map(entries));
    });
    return () => {
      alive = false;
    };
  }, [key, refresh, changed]);
  return docs;
}

const GROUP_HEAD = "px-2 pt-3 pb-0.5 text-xs font-semibold text-text/60";
const ITEM =
  "reader-folders__item flex w-full min-h-[44px] items-center gap-2 rounded-md px-2 py-1 text-start transition-colors";
const ITEM_BUTTON = `${ITEM} cursor-pointer text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary`;

/** Kopf des Buch-Ordners: Cover, Titel, Autoren, Anzahl der Fassungen, „Weiterlesen“. */
export const BookFolderHead: React.FC<{
  book: LibraryBook;
  onContinue: () => void;
}> = ({ book, onContinue }) => {
  const { t } = useTranslation();
  const count = editionsOf(book).length;
  return (
    <div
      className="book-folder-head tts-pages__list flex gap-3 border-b border-mid-gray/20 px-1 pb-3 pt-1"
      data-testid="book-folder-head"
    >
      <span className="book-folder-head__cover block w-14 shrink-0">
        <BookCover book={book} size="sm" />
      </span>
      <div className="book-folder-head__body min-w-0 flex-1">
        <h3 className="break-words text-sm font-semibold text-text">
          {book.title}
        </h3>
        <p className="break-words text-xs text-text/60">
          {[book.authors.join(", "), t("tts.folders.editions", { count })]
            .filter(Boolean)
            .join(" · ")}
        </p>
        <Button
          variant="primary"
          className="book-folder-head__continue reader-folders__btn mt-2"
          icon={<BookOpen width={16} height={16} aria-hidden="true" />}
          onClick={onContinue}
          data-testid="book-continue"
        >
          {t("tts.folders.continue")}
        </Button>
      </div>
    </div>
  );
};

/** Fassung, deren Quelle (Seite) nicht mehr auffindbar ist. Nichts wird gelöscht. */
export const MissingEdition: React.FC<{ edition: Edition }> = ({ edition }) => {
  const { t } = useTranslation();
  return (
    <div
      className={`tts-pages__list ${ITEM} text-text`}
      data-testid="edition-missing"
      data-edition={edition.id}
    >
      <FileText
        width={16}
        height={16}
        className="shrink-0 text-text/60"
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
          <span className="break-words text-sm">
            {editionLang(edition, t("tts.folders.unknownLang"))}
          </span>
          <span className="inline-flex items-center gap-1 rounded border border-orange-500/70 bg-orange-500/10 px-1.5 text-xs font-semibold text-text">
            <AlertTriangle
              width={12}
              height={12}
              className="text-orange-500"
              aria-hidden="true"
            />
            {t("tts.folders.missing")}
          </span>
        </span>
        <span className="block text-xs text-text/60">
          {t("tts.folders.missingHint")}
        </span>
      </span>
    </div>
  );
};

/** Hörbuch- und KI-Dokument-Gruppe unter den Fassungen. */
export const BookFolderGroups: React.FC<{
  book: LibraryBook;
  /** Seiten, die es gibt (die Seitenliste kennt sie). */
  pageIds: Set<string>;
  kiDocs: Map<string, KiDocument[]>;
  /** Geöffnetes KI-Dokument der offenen Seite. */
  activeDoc: { page: string; id: string } | null;
  onOpenPage: (pageId: string) => void;
  onOpenDoc: (pageId: string, docId: string) => void;
}> = ({ book, pageIds, kiDocs, activeDoc, onOpenPage, onOpenDoc }) => {
  const { t } = useTranslation();
  const unknown = t("tts.folders.unknownLang");
  const editions = useMemo(() => editionsOf(book), [book]);
  const main = primaryEdition(book);
  const audio = editions.filter((e) => e.audio);
  const aiRows = editions.flatMap((e) => {
    const page = pageOf(e);
    if (!page || !pageIds.has(page)) return [];
    return (kiDocs.get(page) ?? []).map((doc) => ({ edition: e, page, doc }));
  });
  return (
    <>
      {audio.length > 0 && (
        <section
          aria-label={t("tts.folders.groupAudio")}
          className="tts-pages__list"
        >
          <p className={GROUP_HEAD}>{t("tts.folders.groupAudio")}</p>
          {audio.map((e) => {
            const tracks = tracksOf(e.audio);
            const total = tracks.reduce(
              (sum, x) => sum + (x.duration_ms ?? 0),
              0,
            );
            const ms = total || (e.audio?.duration_ms ?? 0);
            const page = pageOf(e);
            const target = page && pageIds.has(page) ? page : null;
            const body = (
              <>
                <Headphones
                  width={16}
                  height={16}
                  className="shrink-0 text-text/60"
                  aria-hidden="true"
                />
                <span className="min-w-0 flex-1">
                  <span className="block break-words text-sm">
                    {t("tts.folders.audioTitle", {
                      lang: editionLang(e, unknown),
                    })}
                  </span>
                  <span className="block text-xs text-text/60">
                    {t("tts.folders.audioMeta", {
                      count: tracks.length,
                      duration: formatDuration(ms),
                    })}
                  </span>
                </span>
              </>
            );
            return target ? (
              <button
                key={e.id}
                type="button"
                data-testid="book-audio"
                className={ITEM_BUTTON}
                onClick={() => onOpenPage(target)}
              >
                {body}
              </button>
            ) : (
              <div
                key={e.id}
                data-testid="book-audio"
                className={`${ITEM} text-text`}
              >
                {body}
              </div>
            );
          })}
        </section>
      )}
      {aiRows.length > 0 && (
        <section
          aria-label={t("tts.folders.groupAi")}
          className="tts-pages__list"
        >
          <p className={GROUP_HEAD}>{t("tts.folders.groupAi")}</p>
          {aiRows.map(({ edition, page, doc }) => {
            const lang = editionLang(edition, unknown);
            const open = activeDoc?.page === page && activeDoc.id === doc.id;
            return (
              <button
                key={`${page}:${doc.id}`}
                type="button"
                data-testid="book-ai-doc"
                aria-current={open ? "true" : undefined}
                className={`${ITEM_BUTTON} ${open ? "bg-logo-primary/15 text-text" : ""}`}
                onClick={() => onOpenDoc(page, doc.id)}
              >
                <Sparkles
                  width={16}
                  height={16}
                  className="shrink-0 text-text/60"
                  aria-hidden="true"
                />
                <span className="min-w-0 flex-1">
                  <span className="block break-words text-sm">{doc.title}</span>
                  <span className="block text-xs text-text/60">
                    {edition.id === main?.id
                      ? t("tts.folders.aiMetaMain", { lang })
                      : t("tts.folders.aiMeta", { lang })}
                  </span>
                </span>
              </button>
            );
          })}
        </section>
      )}
      <BookNotesGroup bookId={book.id} />
    </>
  );
};

/**
 * Dateien der Gruppe „Notizen und Anhänge“; lädt nach einem Abgleich neu. `failed`:
 * der letzte Ladeversuch scheiterte (eigener Zustand, nicht „keine Dateien“).
 */
function useBookFolderFiles(bookId: string) {
  const [files, setFiles] = useState<BookFolderFile[]>([]);
  const [failed, setFailed] = useState(false);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((n) => n + 1), []);
  useEffect(() => {
    const unlisten = listen("sync-changed", reload);
    return () => {
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [reload]);
  useEffect(() => {
    let alive = true;
    void commands
      .libraryBookFolderFiles(bookId)
      .then((r) => {
        if (!alive) return;
        const ok = r.status === "ok" && Array.isArray(r.data);
        setFailed(!ok);
        setFiles(ok ? r.data : []);
      })
      .catch(() => {
        if (!alive) return;
        setFailed(true);
        setFiles([]);
      });
    return () => {
      alive = false;
    };
  }, [bookId, tick]);
  return { files, failed, reload };
}

const ROLE_ICON: Record<string, typeof FileText> = {
  notiz: NotebookPen,
  anhang: Paperclip,
  transkript: FileText,
};

/**
 * Notizen, Anhänge und Transkripte im Buch-Ordner (lva#365 P3b). Eine Zeile öffnet
 * die Datei im Standardprogramm; „Entfernen“ legt sie in den Papierkorb der
 * Bibliothek. Noch nicht geladene Dateien anderer Geräte stehen gedämpft da.
 */
export const BookNotesGroup: React.FC<{ bookId: string }> = ({ bookId }) => {
  const { t, i18n } = useTranslation();
  const { files, failed, reload } = useBookFolderFiles(bookId);
  // Je Datei höchstens ein Öffnen gleichzeitig (Doppelklick öffnet nicht zweimal).
  const opening = useRef(new Set<string>());
  const [naming, setNaming] = useState(false);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const fail = (e: unknown) =>
    toast.error(t("tts.folders.fileError", { error: String(e) }));

  const openFile = async (rel: string) => {
    if (opening.current.has(rel)) return;
    opening.current.add(rel);
    try {
      const r = await commands.libraryBookFileOpen(bookId, rel).catch((e) => ({
        status: "error" as const,
        error: String(e),
      }));
      if (r.status === "error") fail(r.error);
    } finally {
      opening.current.delete(rel);
    }
  };

  const createNote = async () => {
    const name = title.trim();
    if (!name || busy) return;
    setBusy(true);
    try {
      const r = await commands.libraryBookNoteCreate(bookId, name);
      if (r.status === "error") return fail(r.error);
      setNaming(false);
      setTitle("");
      reload();
      await openFile(r.data);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const addAttachments = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const picked = await openDialog({ multiple: true, directory: false });
      const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
      if (paths.length === 0) return;
      const r = await commands.libraryBookAttachmentsAdd(bookId, paths);
      if (r.status === "error") return fail(r.error);
      reload();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const remove = async (f: BookFolderFile) => {
    const ok = await ask(t("tts.folders.removeAsk", { name: f.name }), {
      title: t("tts.folders.removeTitle"),
      kind: "warning",
      okLabel: t("tts.folders.remove"),
      cancelLabel: t("common.cancel"),
    }).catch(() => false);
    if (!ok) return;
    const r = await commands
      .libraryBookFileRemove(bookId, f.rel)
      .catch((e) => ({ status: "error" as const, error: String(e) }));
    if (r.status === "error") fail(r.error);
    reload();
  };

  const meta = (f: BookFolderFile): string => {
    const kind =
      f.role === "transkript"
        ? f.edition
          ? t("tts.folders.roleTranscriptOf", { edition: f.edition })
          : t("tts.folders.roleTranscript")
        : f.role === "notiz"
          ? t("tts.folders.roleNote")
          : t("tts.folders.roleAttachment");
    const parts = [kind];
    if (f.held) parts.push(t("tts.folders.held"));
    else if (f.folder_missing) parts.push(t("tts.folders.folderMissing"));
    else if (f.pending) parts.push(t("tts.folders.pending"));
    else if (f.bytes > 0)
      parts.push(formatBytes(f.bytes, i18n.language || "de"));
    return parts.join(" · ");
  };

  return (
    <section
      aria-label={t("tts.folders.groupFiles")}
      className="tts-pages__list"
      data-testid="book-notes"
    >
      <p className={GROUP_HEAD}>{t("tts.folders.groupFiles")}</p>
      {failed ? (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-2 px-2 py-1 text-xs text-text/80"
          data-testid="book-notes-error"
        >
          <AlertTriangle
            width={14}
            height={14}
            className="shrink-0 text-orange-500"
            aria-hidden="true"
          />
          <span className="min-w-0 flex-1">{t("tts.folders.loadError")}</span>
          <Button
            variant="secondary"
            className="reader-folders__btn min-h-[44px]"
            onClick={reload}
          >
            {t("tts.folders.reload")}
          </Button>
        </div>
      ) : (
        files.length === 0 && (
          <p className="px-2 py-1 text-xs text-text/60">
            {t("tts.folders.noFiles")}
          </p>
        )
      )}
      {files.map((f) => {
        // Nur vorhandene Dateien lassen sich öffnen; entfernen geht auch, wenn ihr Ordner
        // fehlt (ausdrückliche Löschung), nicht aber bei Zurückgehaltenem oder Erwartetem.
        const openable = !f.pending && !f.held && !f.folder_missing;
        const removable = !f.pending && !f.held;
        const Icon = f.pending
          ? CloudDownload
          : f.held || f.folder_missing
            ? AlertTriangle
            : (ROLE_ICON[f.role] ?? FileText);
        const body = (
          <>
            <Icon
              width={16}
              height={16}
              className="shrink-0 text-text/60"
              aria-hidden="true"
            />
            <span className="min-w-0 flex-1">
              <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                <span className="break-words text-sm">{f.name}</span>
                {f.conflict && (
                  <span className="inline-flex items-center gap-1 rounded border border-orange-500/70 bg-orange-500/10 px-1.5 text-xs font-semibold text-text">
                    <AlertTriangle
                      width={12}
                      height={12}
                      className="text-orange-500"
                      aria-hidden="true"
                    />
                    {t("tts.folders.conflict")}
                  </span>
                )}
              </span>
              <span className="block text-xs text-text/60">{meta(f)}</span>
            </span>
          </>
        );
        return (
          <div
            key={f.rel}
            className="flex items-center gap-1"
            data-testid="book-file"
            data-rel={f.rel}
          >
            {!openable ? (
              <div className={`${ITEM} min-w-0 flex-1 text-text/70`}>
                {body}
              </div>
            ) : (
              <button
                type="button"
                className={`${ITEM_BUTTON} min-w-0 flex-1`}
                onClick={() => void openFile(f.rel)}
                title={t("tts.folders.openFile")}
              >
                {body}
              </button>
            )}
            {removable && (
              <button
                type="button"
                className="flex min-h-[44px] min-w-[44px] shrink-0 cursor-pointer items-center justify-center rounded-md text-text/60 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary"
                aria-label={t("tts.folders.removeFile", { name: f.name })}
                title={t("tts.folders.remove")}
                data-testid="book-file-remove"
                onClick={() => void remove(f)}
              >
                <Trash2 width={16} height={16} aria-hidden="true" />
              </button>
            )}
          </div>
        );
      })}
      {naming ? (
        <form
          className="flex flex-wrap items-center gap-2 px-2 py-1"
          onSubmit={(e) => {
            e.preventDefault();
            void createNote();
          }}
        >
          <input
            autoFocus
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setNaming(false);
                setTitle("");
              }
            }}
            aria-label={t("tts.folders.noteTitle")}
            placeholder={t("tts.folders.noteTitle")}
            maxLength={120}
            className="min-h-[44px] min-w-0 flex-1 rounded-md border border-mid-gray/40 bg-background px-2 text-sm text-text"
            data-testid="book-note-title"
          />
          <Button
            type="submit"
            variant="primary"
            className="reader-folders__btn min-h-[44px]"
            disabled={!title.trim() || busy}
          >
            {t("tts.folders.noteCreate")}
          </Button>
        </form>
      ) : (
        <div className="flex flex-wrap gap-2 px-2 py-1">
          <Button
            variant="secondary"
            className="reader-folders__btn min-h-[44px]"
            icon={<FilePlus width={16} height={16} aria-hidden="true" />}
            onClick={() => setNaming(true)}
            data-testid="book-note-new"
          >
            {t("tts.folders.newNote")}
          </Button>
          <Button
            variant="secondary"
            className="reader-folders__btn min-h-[44px]"
            icon={<Paperclip width={16} height={16} aria-hidden="true" />}
            onClick={() => void addAttachments()}
            disabled={busy}
            data-testid="book-attach"
          >
            {t("tts.folders.addAttachment")}
          </Button>
        </div>
      )}
    </section>
  );
};

export { GROUP_HEAD };
