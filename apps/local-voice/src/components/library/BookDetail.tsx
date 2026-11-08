import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  BookOpen,
  Check,
  Download,
  Headphones,
  ImageIcon,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Sparkles,
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
import { CoverDialog } from "./CoverDialog";
import { TitleEditor } from "./TitleEditor";
import { Formats, jobLabel, ProgressBar } from "./LibraryPage";
import type { TranscriptJob } from "./useLibrary";
import { MetadataPicker } from "./MetadataPicker";
import { KurzbuchCard } from "./KurzbuchCard";
import { EditionsSection, LastActive } from "./EditionsSection";
import { BookFiles } from "./BookFiles";
import "./fassungen.css";
import type { KiCreateTab } from "./KiCreateDialog";
import {
  fileNameOf,
  formatDuration,
  isFinished,
  kindOfFile,
  openInReader,
  tracksOf,
  type BookView,
} from "@/lib/library/books";
import { groupAudioFiles } from "@/lib/library/audioGroups";
import {
  drmMessage,
  importAudioParts,
  resumeTranscription,
  transcribeTracks,
} from "@/lib/library/importFile";
import { jobsOfTrack } from "@/lib/library/sections";
import {
  audioProgressKey,
  loadProgress,
  progressFraction,
  removeProgress,
} from "@/lib/library/progress";
import {
  audioKeyOf,
  editionsOf,
  hasAudio,
  langName,
  langShort,
  otherAudioEdition,
} from "@/lib/library/editions";

type Tab = "overview" | "versions" | "files" | "metadata";

interface Props {
  view: BookView;
  job?: TranscriptJob;
  onBack(): void;
  onPlay(): void;
  onListen(): void;
  /** Hören und gleichzeitig den Text mitlesen. */
  onListenFollow(): void;
  /** Hörbuch einer bestimmten Sprachfassung abspielen. */
  onListenEdition?(editionId: string): void;
  onToggleFinished(): void;
  /** Ändert den frisch gelesenen Stand des Buchs (siehe useLibrary.save). */
  onSave(
    id: string,
    update: (fresh: LibraryBook) => LibraryBook,
  ): Promise<LibraryBook | null>;
  /** Entfernen: Rückfrage und Rückgängig liegen bei der Bibliothek. */
  onRemove(): void;
  onChanged(): Promise<void>;
  /** Beim Öffnen gleich den Titel bearbeiten (⋮ → „Titel bearbeiten“). */
  editTitle?: boolean;
  /** lva#236: „Mit KI erstellen“ (Übersetzung, Kurzbuch, KI-Dokument). */
  onCreate?(tab: KiCreateTab): void;
  /** lva#236: Fassungen exportieren. */
  onExport?(): void;
  /** lva#344: „Sprachfassung hinzufügen …“ (öffnet „Hinzufügen“). */
  onAddEdition?(): void;
  /** lva#344: Kapitel zweier Sprachfassungen zuordnen. */
  onAlign?(pair: { a: string; b: string }): void;
  /** Gescheiterte Transkription erneut anstoßen (useLibrary.retryTranscript). */
  onRetryTranscript?(): Promise<void>;
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
  onListenFollow,
  onListenEdition,
  onToggleFinished,
  onSave,
  onRemove,
  onChanged,
  editTitle = false,
  onCreate,
  onExport,
  onAddEdition,
  onAlign,
  onRetryTranscript,
}) => {
  const { t } = useTranslation();
  const { book, shown } = view;
  const multi = editionsOf(book).length > 1;
  const readingLang = view.edition
    ? langName(view.edition.lang) || langShort(view.edition.lang)
    : "";
  // Die aktive Fassung hat kein Hörbuch, eine andere schon: im Kopf anbieten.
  const audioEdition =
    multi && !view.audioSource && onListenEdition
      ? otherAudioEdition(book, view.edition)
      : null;
  const audioLang = audioEdition
    ? langName(audioEdition.lang) || langShort(audioEdition.lang)
    : "";
  const audioRec = audioEdition
    ? loadProgress(audioKeyOf(book, audioEdition))
    : null;
  const audioStarted =
    !!audioRec && audioRec.offset > 0 && progressFraction(audioRec) < 0.995;
  const [tab, setTab] = useState<Tab>("overview");
  const [draft, setDraft] = useState(book);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [coverOpen, setCoverOpen] = useState(false);
  const [titleEditing, setTitleEditing] = useState(editTitle);
  useEffect(() => setTitleEditing(editTitle), [book.id, editTitle]);
  // Editor zu: Fokus zurück auf den Stift (Tastatur, Bildschirmleser).
  const pencil = useRef<HTMLButtonElement>(null);
  const wasEditing = useRef(titleEditing);
  useEffect(() => {
    if (wasEditing.current && !titleEditing) pencil.current?.focus();
    wasEditing.current = titleEditing;
  }, [titleEditing]);
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

  /** Hörbuch ersetzen oder (append) weitere Teile hinter die vorhandenen setzen. */
  const addAudio = (append: boolean) =>
    run("audio", async () => {
      const picked = await openFile({
        multiple: true,
        filters: [
          {
            name: t("library.audioFiles"),
            extensions: ["mp3", "m4b", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"],
          },
        ],
      });
      const all = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
      if (!all.length) return;
      const drm = all.filter((p) => kindOfFile(p) === "drm");
      if (drm.length) toast.error(drmMessage(fileNameOf(drm[0])));
      const audio = all.filter((p) => kindOfFile(p) === "audio");
      if (!audio.length) return;
      // Hörreihenfolge: Disc, Track, Name.
      const paths = groupAudioFiles(audio).flatMap((g) => g.paths);
      await importAudioParts(book, paths, append, (m) => toast.error(m));
      await onChanged();
    });

  const transcribe = () =>
    run("transcribe", async () => {
      if (!book.audio_path || !book.audio) return;
      // Ein Hörbuch, keine Gesprächsaufnahme: es gibt niemanden, dessen
      // Einwilligung fehlen könnte. Ein Auftrag je Teil.
      await transcribeTracks(book.id, (m) => toast.error(m));
      await onChanged();
    });

  /**
   * Nur fehlende oder fehlgeschlagene Teile und Abschnitte neu starten; ist
   * nur die Übernahme gescheitert, wird sie erneut versucht.
   */
  const resume = () =>
    run("transcribe", async () => {
      if (onRetryTranscript) await onRetryTranscript();
      else await resumeTranscription(book.id, (m) => toast.error(m));
      await onChanged();
    });

  const missingParts =
    !book.text && tracksOf(book.audio).some((x) => !jobsOfTrack(x).length);
  const canResume =
    job !== undefined &&
    (missingParts ||
      job.failed > 0 ||
      job.status === "failed" ||
      job.status === "missing");

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
        <button
          type="button"
          className="library-hero__cover"
          onClick={() => setCoverOpen(true)}
          aria-label={t("library.cover.change")}
          title={t("library.cover.change")}
          data-testid="library-detail-cover"
        >
          <BookCover book={book} size="lg" />
          <span className="library-hero__cover-edit" aria-hidden="true">
            <ImageIcon size={14} />
            {t("library.cover.changeShort")}
          </span>
        </button>
        <div style={{ minWidth: 0 }}>
          <div className="library-meta">
            {[book.language, book.publish_date].filter(Boolean).join(" · ")}
          </div>
          {titleEditing ? (
            <TitleEditor
              key={book.id}
              title={book.title}
              onCancel={() => setTitleEditing(false)}
              onSave={async (title) => {
                const saved = await onSave(book.id, (fresh) => ({ ...fresh, title }));
                if (!saved) return;
                // Ein offenes Metadatenformular darf den Titel nicht zurücksetzen.
                setDraft((d) => ({ ...d, title }));
                setTitleEditing(false);
                toast(t("library.saved"));
              }}
            />
          ) : (
            <div className="library-hero__title-row">
              <h2 className="library-hero__title">{book.title}</h2>
              <button
                ref={pencil}
                type="button"
                className="library-hero__title-edit"
                onClick={() => setTitleEditing(true)}
                aria-label={t("library.titleEdit.action")}
                title={t("library.titleEdit.action")}
                data-testid="library-title-edit"
              >
                <Pencil size={16} aria-hidden="true" />
              </button>
            </div>
          )}
          {book.subtitle && <div className="library-hero__sub">{book.subtitle}</div>}
          {book.authors.length > 0 && (
            <div className="library-hero__sub">
              {t("library.by", { authors: book.authors.join(", ") })}
            </div>
          )}
          <LastActive book={book} />
          <div style={{ margin: "10px 0" }}>
            <Formats
              view={view}
              job={job}
              otherAudio={audioLang || undefined}
              otherAudioLocal={!!audioEdition && hasAudio(book, audioEdition)}
            />
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
              {multi && readingLang ? ` · ${readingLang}` : ""}
            </Button>
          )}
          {shown.text && (
            <Button
              size="lg"
              variant={view.audioSource ? undefined : "primary"}
              icon={<BookOpen size={16} aria-hidden="true" />}
              onClick={() => openInReader(shown.text!.page_id, false)}
              data-testid="library-detail-read"
            >
              {!view.audioSource && view.fraction > 0 && !isFinished(view) ? t("library.continueRead") : t("library.read")}
            </Button>
          )}
          {shown.text && shown.audio_path && (
            <Button
              size="lg"
              icon={<Headphones size={16} aria-hidden="true" />}
              onClick={onListenFollow}
              data-testid="library-detail-listen-follow"
            >
              {t("library.listenFollow")}
            </Button>
          )}
          {shown.audio_path && view.resumeWith !== "audio" && (
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
          {audioEdition && onListenEdition && (
            <Button
              size="lg"
              icon={<Headphones size={16} aria-hidden="true" />}
              onClick={() => onListenEdition(audioEdition.id)}
              data-testid="library-detail-listen-edition"
            >
              {t(
                audioStarted
                  ? "library.sf.continueEdition"
                  : "library.sf.listenEdition",
                { lang: audioLang },
              )}
            </Button>
          )}
          <Button
            size="lg"
            icon={<Check size={16} aria-hidden="true" />}
            onClick={onToggleFinished}
          >
            {isFinished(view) ? t("library.markUnfinished") : t("library.markFinished")}
          </Button>
          {onAddEdition && (
            <Button
              size="lg"
              icon={<Plus size={16} aria-hidden="true" />}
              onClick={onAddEdition}
              data-testid="library-detail-add-edition"
            >
              {t("library.sf.add")}
            </Button>
          )}
          {(onCreate || onExport) && (
            <div className="library-actions__pair">
              {onCreate && (
                <Button
                  size="lg"
                  icon={<Sparkles size={16} aria-hidden="true" />}
                  onClick={() => onCreate("translation")}
                  title={t("library.ki.title")}
                  aria-label={t("library.ki.title")}
                  data-testid="library-detail-ki"
                >
                  {t("library.ki.short")}
                </Button>
              )}
              {onExport && (
                <Button
                  size="lg"
                  icon={<Download size={16} aria-hidden="true" />}
                  onClick={onExport}
                  data-testid="library-detail-export"
                >
                  {t("library.export.short")}
                </Button>
              )}
            </div>
          )}
        </div>
      </div>

      {onAlign && (
        <EditionsSection
          book={book}
          onChanged={onChanged}
          onAlign={onAlign}
          onExport={onExport}
          onListen={onListenEdition}
        />
      )}

      <TabList<Tab>
        tabs={[
          { id: "overview", label: t("library.tab.overview") },
          { id: "versions", label: t("library.tab.versions") },
          { id: "files", label: t("library.tab.files") },
          { id: "metadata", label: t("library.tab.metadata") },
        ]}
        value={tab}
        onChange={setTab}
        ariaLabel={t("library.tabs")}
      />

      {tab === "overview" && (
        <div className="library-detail">
          <KurzbuchCard
            book={book}
            transcribing={view.transcription === "running" || job !== undefined}
            busy={busy !== null}
            onTranscribe={() => void transcribe()}
            job={job}
            onRetry={canResume ? () => void resume() : undefined}
          />
          {book.description ? (
            <p className="library-blurb">{book.description}</p>
          ) : (
            <p className="library-meta">{t("library.noDescription")}</p>
          )}
        </div>
      )}

      {tab === "files" && <BookFiles book={book} />}

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
                ? jobLabel(t, job)
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
                {job && (
                  <span className="library-meta">{jobLabel(t, job)}</span>
                )}
                {canResume && (
                  <Button onClick={() => void resume()} disabled={busy !== null}>
                    {t("library.transcribeMissing")}
                  </Button>
                )}
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
              <>
                <span className="library-meta">{jobLabel(t, job)}</span>
                {canResume && (
                  <Button onClick={() => void resume()} disabled={busy !== null}>
                    {t("library.transcribeMissing")}
                  </Button>
                )}
              </>
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
                  {tracksOf(book.audio).length > 1
                    ? t("library.importedParts", { count: tracksOf(book.audio).length })
                    : t("library.importedAudio", { file: book.audio?.file ?? "" })}
                </span>
                {tracksOf(book.audio).length > 1 && (
                  <ol className="library-parts-list" data-testid="library-detail-parts">
                    {tracksOf(book.audio).map((track, i) => (
                      <li key={track.file}>
                        <span className="library-parts__no">{i + 1}</span>
                        <span className="library-parts__title">{track.title || track.file}</span>
                        <span className="library-meta">
                          {track.duration_ms ? formatDuration(track.duration_ms) : ""}
                        </span>
                      </li>
                    ))}
                  </ol>
                )}
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
              onClick={() => void addAudio(false)}
              disabled={busy !== null}
            >
              {book.audio_path ? t("library.replaceAudio") : t("library.addAudio")}
            </Button>
            {book.audio_path && (
              <Button
                variant="ghost"
                icon={<Upload size={16} aria-hidden="true" />}
                onClick={() => void addAudio(true)}
                disabled={busy !== null}
                data-testid="library-detail-append"
              >
                {t("library.appendAudio")}
              </Button>
            )}
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
              onClick={() => setCoverOpen(true)}
              disabled={busy !== null}
              data-testid="library-meta-cover"
            >
              {t("library.cover.change")}
            </Button>
            <span className="library-bar__spacer" />
            <Button
              variant="danger-ghost"
              icon={<Trash2 size={16} aria-hidden="true" />}
              onClick={onRemove}
              disabled={busy !== null}
              data-testid="library-detail-remove"
            >
              {t("library.remove.action")}
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
      <CoverDialog
        book={coverOpen ? book : null}
        onClose={() => setCoverOpen(false)}
        onChanged={async () => {
          await onChanged();
          toast(t("library.cover.done"));
        }}
      />
    </div>
  );
};
