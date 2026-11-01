import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  BookOpen,
  Check,
  Headphones,
  ImageIcon,
  Play,
  RefreshCw,
  Trash2,
  Upload,
} from "lucide-react";
import { commands, type LibraryBook } from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Textarea } from "../ui/Textarea";
import { TabList } from "../ui/TabList";
import { Dialog } from "../ui/Dialog";
import { BookCover } from "./BookCover";
import { Formats, ProgressBar } from "./LibraryPage";
import type { TranscriptJob } from "./useLibrary";
import { MetadataPicker } from "./MetadataPicker";
import {
  formatDuration,
  isFinished,
  openInReader,
  type BookView,
} from "@/lib/library/books";

type Tab = "overview" | "versions" | "metadata";

interface Props {
  view: BookView;
  job?: TranscriptJob;
  onBack(): void;
  onPlay(): void;
  onListen(): void;
  onToggleFinished(): void;
  /** Ändert den frisch gelesenen Stand des Buchs (siehe useLibrary.save). */
  onSave(
    id: string,
    update: (fresh: LibraryBook) => LibraryBook,
  ): Promise<LibraryBook | null>;
  onDeleted(): Promise<void>;
  onChanged(): Promise<void>;
}

/**
 * Produktseite eines Buchs (Variante D): großes Cover, Metadaten, Aktionen,
 * darunter Übersicht, Fassungen und Metadaten zum Bearbeiten.
 */
export const BookDetail: React.FC<Props> = ({
  view,
  job,
  onBack,
  onPlay,
  onListen,
  onToggleFinished,
  onSave,
  onDeleted,
  onChanged,
}) => {
  const { t } = useTranslation();
  const { book } = view;
  const [tab, setTab] = useState<Tab>("overview");
  const [draft, setDraft] = useState(book);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // Neues Buch: Formular zurücksetzen. Neu geladener Stand desselben Buchs
  // (z. B. Cover fertig) nur übernehmen, solange nichts getippt ist.
  useEffect(() => {
    setDraft(book);
    setDirty(false);
  }, [book.id]);
  useEffect(() => {
    if (!dirty) setDraft(book);
  }, [book, dirty]);
  const edit = (patch: Partial<LibraryBook>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setDirty(true);
  };

  const run = async (label: string, task: () => Promise<void>) => {
    setBusy(label);
    try {
      await task();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const pickCover = () =>
    run("cover", async () => {
      const picked = await openFile({
        multiple: false,
        filters: [
          { name: t("library.coverFiles"), extensions: ["jpg", "jpeg", "png", "webp", "epub"] },
        ],
      });
      if (typeof picked !== "string") return;
      const r = await commands.libraryCoverFromFile(book.id, picked);
      if (r.status === "error") throw new Error(r.error);
      await onChanged();
    });

  const addAudio = () =>
    run("audio", async () => {
      const picked = await openFile({
        multiple: false,
        filters: [
          {
            name: t("library.audioFiles"),
            extensions: ["mp3", "m4b", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"],
          },
        ],
      });
      if (typeof picked !== "string") return;
      const r = await commands.libraryAudioImport(book.id, picked);
      if (r.status === "error") throw new Error(r.error);
      await onChanged();
    });

  const transcribe = () =>
    run("transcribe", async () => {
      if (!book.audio_path || !book.audio) return;
      // Ein Hörbuch, keine Gesprächsaufnahme: es gibt niemanden, dessen
      // Einwilligung fehlen könnte.
      const r = await commands.meetingsImportFile(book.audio_path, true, null);
      if (r.status === "error") throw new Error(r.error);
      await onSave(book.id, (fresh) => ({
        ...fresh,
        audio: fresh.audio ? { ...fresh.audio, transcript_job: r.data } : fresh.audio,
      }));
    });

  const remove = () =>
    run("delete", async () => {
      setConfirmDelete(false);
      const r = await commands.libraryDelete(book.id);
      if (r.status === "error") throw new Error(r.error);
      await onDeleted();
    });

  const saveDraft = () =>
    run("save", async () => {
      await onSave(book.id, (fresh) => ({
        ...fresh,
        title: draft.title,
        subtitle: draft.subtitle,
        authors: draft.authors.map((a) => a.trim()).filter(Boolean),
        language: draft.language,
        isbn: draft.isbn,
        publisher: draft.publisher,
        publish_date: draft.publish_date,
        pages: draft.pages,
        description: draft.description,
        metadata_source: "manual",
        audio: fresh.audio
          ? { ...fresh.audio, narrator: draft.audio?.narrator ?? null }
          : fresh.audio,
      }));
      setDirty(false);
      toast(t("library.saved"));
    });

  const rec = view.progress;
  const progressText = isFinished(view)
    ? t("library.status.finished")
    : rec?.unit === "ms" && rec.length > 0
      ? t("library.status.leftOf", {
          left: formatDuration(rec.length - rec.offset),
          total: formatDuration(rec.length),
        })
      : t("library.status.percent", { percent: Math.round(view.fraction * 100) });

  const meta: [string, string | null | undefined][] = [
    [t("library.field.authors"), book.authors.join(", ")],
    [t("library.field.narrator"), book.audio?.narrator],
    [t("library.field.publisher"), book.publisher],
    [t("library.field.date"), book.publish_date],
    [t("library.field.isbn"), book.isbn],
    [t("library.field.pages"), book.pages ? String(book.pages) : null],
    [t("library.field.language"), book.language],
    [
      t("library.field.source"),
      book.metadata_source ? t(`library.source.${book.metadata_source}`) : null,
    ],
  ];

  return (
    <div className="library-detail" data-testid="library-detail">
      <div className="library-bar">
        <Button
          variant="ghost"
          icon={<ArrowLeft size={16} aria-hidden="true" />}
          onClick={onBack}
        >
          {t("library.back")}
        </Button>
      </div>
      <div className="library-hero">
        <BookCover book={book} size="lg" />
        <div style={{ minWidth: 0 }}>
          <div className="library-meta">
            {[book.language, book.publish_date].filter(Boolean).join(" · ")}
          </div>
          <h2 className="library-hero__title">{book.title}</h2>
          {book.subtitle && <div className="library-hero__sub">{book.subtitle}</div>}
          {book.authors.length > 0 && (
            <div className="library-hero__sub">
              {t("library.by", { authors: book.authors.join(", ") })}
            </div>
          )}
          <div style={{ margin: "10px 0" }}>
            <Formats view={view} job={job} />
          </div>
          <dl className="library-kv">
            {meta
              .filter(([, v]) => v)
              .map(([k, v]) => (
                <React.Fragment key={k}>
                  <dt>{k}</dt>
                  <dd>{v}</dd>
                </React.Fragment>
              ))}
          </dl>
        </div>
        <div className="library-actions">
          {view.audioSource && (
            <Button
              variant="primary"
              size="lg"
              icon={<Play size={16} aria-hidden="true" />}
              onClick={onPlay}
              data-testid="library-detail-play"
            >
              {view.fraction > 0 && !isFinished(view)
                ? t("library.continuePlay")
                : t("library.play")}
            </Button>
          )}
          {book.text && (
            <Button
              size="lg"
              variant={view.audioSource ? undefined : "primary"}
              icon={<BookOpen size={16} aria-hidden="true" />}
              onClick={() => openInReader(book.text!.page_id, false)}
              data-testid="library-detail-read"
            >
              {!view.audioSource && view.fraction > 0 && !isFinished(view) ? t("library.continueRead") : t("library.read")}
            </Button>
          )}
          {book.audio_path && view.resumeWith !== "audio" && (
            <Button
              size="lg"
              icon={<Headphones size={16} aria-hidden="true" />}
              onClick={onListen}
            >
              {t("library.listenOriginal")}
            </Button>
          )}
          <ProgressBar
            fraction={view.fraction}
            label={t("library.progressLabel", { title: book.title })}
          />
          <div className="library-meta">{progressText}</div>
          <Button
            size="lg"
            icon={<Check size={16} aria-hidden="true" />}
            onClick={onToggleFinished}
          >
            {isFinished(view) ? t("library.markUnfinished") : t("library.markFinished")}
          </Button>
        </div>
      </div>

      <TabList<Tab>
        tabs={[
          { id: "overview", label: t("library.tab.overview") },
          { id: "versions", label: t("library.tab.versions") },
          { id: "metadata", label: t("library.tab.metadata") },
        ]}
        value={tab}
        onChange={setTab}
        ariaLabel={t("library.tabs")}
      />

      {tab === "overview" && (
        <div className="library-detail">
          {book.description ? (
            <p className="library-blurb">{book.description}</p>
          ) : (
            <p className="library-meta">{t("library.noDescription")}</p>
          )}
        </div>
      )}

      {tab === "versions" && (
        <div className="library-versions">
          <dl className="library-kv library-status" data-testid="library-status">
            <dt>{t("library.statusAudio")}</dt>
            <dd>{t(`library.audioSource.${view.audioSource ?? "none"}`)}</dd>
            <dt>{t("library.statusNarration")}</dt>
            <dd>
              {view.audioSource === "narrated"
                ? t("library.narration.done")
                : book.text
                  ? t("library.narration.possible")
                  : t("library.narration.none")}
            </dd>
            <dt>{t("library.statusTranscription")}</dt>
            <dd>
              {view.transcription === "running" && job
                ? t(`library.job.${job.status}`, {
                    defaultValue: t("library.job.processing"),
                  })
                : t(`library.transcription.${view.transcription}`)}
            </dd>
          </dl>
          <section className="library-version" aria-labelledby="ver-text">
            <h3 id="ver-text">
              <BookOpen size={14} aria-hidden="true" />
              {t("library.textVersion")}
            </h3>
            {book.text ? (
              <>
                <span className="library-meta">
                  {t(`library.origin.${book.text.origin}`)}
                  {book.text.source_name ? ` · ${book.text.source_name}` : ""}
                </span>
                <span className="library-meta">
                  {view.audio === "ok" && !book.audio_path
                    ? t("library.generatedAudio")
                    : t("library.generateHint")}
                </span>
                <Button onClick={() => openInReader(book.text!.page_id, false)}>
                  {t("library.openReader")}
                </Button>
              </>
            ) : job ? (
              <span className="library-meta">
                {t(`library.job.${job.status}`, {
                  defaultValue: t("library.job.processing"),
                })}
              </span>
            ) : book.audio_path ? (
              <>
                <span className="library-meta">{t("library.transcribeHint")}</span>
                <Button onClick={() => void transcribe()} disabled={busy !== null}>
                  {t("library.transcribe")}
                </Button>
              </>
            ) : (
              <span className="library-meta">{t("library.noText")}</span>
            )}
          </section>
          <section className="library-version" aria-labelledby="ver-audio">
            <h3 id="ver-audio">
              <Headphones size={14} aria-hidden="true" />
              {t("library.audioVersion")}
            </h3>
            {book.audio_path ? (
              <>
                <span className="library-meta">
                  {t("library.importedAudio", { file: book.audio?.file ?? "" })}
                </span>
                <Button onClick={onListen}>{t("library.listen")}</Button>
              </>
            ) : (
              <span className="library-meta">
                {view.audio === "ok" ? t("library.generatedAudio") : t("library.noAudio")}
              </span>
            )}
            <Button
              variant="ghost"
              icon={<Upload size={16} aria-hidden="true" />}
              onClick={() => void addAudio()}
              disabled={busy !== null}
            >
              {book.audio_path ? t("library.replaceAudio") : t("library.addAudio")}
            </Button>
          </section>
        </div>
      )}

      {tab === "metadata" && (
        <div className="library-detail">
          <div className="library-form">
            <label>
              {t("library.field.title")}
              <Input
                value={draft.title}
                onChange={(e) => edit({ title: e.target.value })}
              />
            </label>
            <label>
              {t("library.field.subtitle")}
              <Input
                value={draft.subtitle ?? ""}
                onChange={(e) => edit({ subtitle: e.target.value || null })}
              />
            </label>
            <label>
              {t("library.field.authors")}
              <Input
                value={draft.authors.join(", ")}
                onChange={(e) =>
                  edit({ authors: e.target.value.split(/\s*[,;]\s*/) })
                }
              />
            </label>
            <label>
              {t("library.field.narrator")}
              <Input
                value={draft.audio?.narrator ?? ""}
                disabled={!draft.audio}
                onChange={(e) =>
                  draft.audio &&
                  edit({
                    audio: { ...draft.audio, narrator: e.target.value || null },
                  })
                }
              />
            </label>
            <label>
              {t("library.field.isbn")}
              <Input
                value={draft.isbn ?? ""}
                onChange={(e) => edit({ isbn: e.target.value || null })}
              />
            </label>
            <label>
              {t("library.field.publisher")}
              <Input
                value={draft.publisher ?? ""}
                onChange={(e) => edit({ publisher: e.target.value || null })}
              />
            </label>
            <label>
              {t("library.field.date")}
              <Input
                value={draft.publish_date ?? ""}
                onChange={(e) =>
                  edit({ publish_date: e.target.value || null })
                }
              />
            </label>
            <label>
              {t("library.field.language")}
              <Input
                value={draft.language ?? ""}
                onChange={(e) => edit({ language: e.target.value || null })}
              />
            </label>
          </div>
          <label className="library-form" style={{ display: "flex" }}>
            {t("library.field.description")}
            <Textarea
              value={draft.description ?? ""}
              onChange={(e) =>
                edit({ description: e.target.value || null })
              }
            />
          </label>
          <div className="library-bar">
            <Button variant="primary" onClick={() => void saveDraft()} disabled={busy !== null}>
              {t("library.save")}
            </Button>
            <Button
              icon={<ImageIcon size={16} aria-hidden="true" />}
              onClick={() => void pickCover()}
              disabled={busy !== null}
            >
              {t("library.coverFromFile")}
            </Button>
            {book.cover_source !== "generated" && (
              <Button
                variant="ghost"
                onClick={() =>
                  void run("cover", async () => {
                    const r = await commands.libraryCoverClear(book.id);
                    if (r.status === "error") throw new Error(r.error);
                    await onChanged();
                  })
                }
                disabled={busy !== null}
              >
                {t("library.coverGenerated")}
              </Button>
            )}
            <span className="library-bar__spacer" />
            <Button
              variant="danger-ghost"
              icon={<Trash2 size={16} aria-hidden="true" />}
              onClick={() => setConfirmDelete(true)}
              disabled={busy !== null}
            >
              {t("library.delete")}
            </Button>
          </div>
          <section aria-labelledby="lib-refetch">
            <h3 id="lib-refetch" className="library-section__head">
              <RefreshCw size={14} aria-hidden="true" />
              {t("library.refetch")}
              <small>{t("library.privacyHint")}</small>
            </h3>
            <MetadataPicker
              title={book.title}
              authors={book.authors}
              isbn={book.isbn}
              onApply={(patch, coverId) =>
                run("meta", async () => {
                  const saved = await onSave(book.id, (fresh) => ({ ...fresh, ...patch }));
                  if (saved && (coverId || patch.isbn)) {
                    const r = await commands.libraryCoverFetch(
                      saved.id,
                      coverId ?? null,
                      patch.isbn ?? null,
                    );
                    if (r.status === "error") toast.error(r.error);
                  }
                  await onChanged();
                  toast(t("library.metadataApplied"));
                })
              }
            />
          </section>
        </div>
      )}
      <Dialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("library.deleteTitle", { title: book.title })}
        closeLabel={t("library.cancel")}
        footer={
          <>
            <Button onClick={() => setConfirmDelete(false)}>{t("library.cancel")}</Button>
            <Button
              variant="danger"
              onClick={() => void remove()}
              data-testid="library-delete-confirm"
            >
              {t("library.delete")}
            </Button>
          </>
        }
      >
        <p>{t("library.deleteBody")}</p>
      </Dialog>
    </div>
  );
};
