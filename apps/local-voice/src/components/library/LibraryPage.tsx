import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { BookOpen, Headphones, MoreVertical, Play, Plus } from "lucide-react";
import type { LibraryBook } from "@/bindings";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { ActionMenu } from "../ui/ActionMenu";
import { Select } from "../ui/Select";
import { BookCover } from "./BookCover";
import { BookDetail } from "./BookDetail";
import { AddBookDialog } from "./AddBookDialog";
import { AudioBar } from "./AudioBar";
import { useLibraryDrop } from "./useLibraryDrop";
import { useLibrary, type TranscriptJob } from "./useLibrary";
import {
  formatDuration,
  isFinished,
  isStarted,
  matchesFilter,
  matchesSearch,
  openInReader,
  sortViews,
  viewOf,
  type BookView,
  type Filter,
  type Sort,
} from "@/lib/library/books";
import { setFinished } from "@/lib/library/progress";
import { usePersistentState } from "@/hooks/usePersistentState";
import "./library.css";

const FILTERS: Filter[] = [
  "all",
  "notStarted",
  "inProgress",
  "finished",
  "noAudio",
  "working",
];
const SORTS: Sort[] = ["recent", "added", "title", "author"];
const isSort = (v: string) => (SORTS as string[]).includes(v);
const isFilter = (v: string) => (FILTERS as string[]).includes(v);

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
          ? t(`library.job.${job?.status ?? "queued"}`, {
              defaultValue: t("library.job.processing"),
            })
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
  const { drop, working, dialog: dropDialog } = useLibraryDrop(books, reload);
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
  const [adding, setAdding] = useState(false);
  const [listening, setListening] = useState<LibraryBook | null>(null);

  const views = useMemo(
    () =>
      (books ?? []).map((b) =>
        viewOf(b, b.text ? !!pageAudio[b.text.page_id] : false),
      ),
    // tick: Fortschritt aus dem localStorage neu lesen.
    [books, pageAudio, tick],
  );
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
  const continueViews = sortViews(views.filter(isStarted), "recent").slice(
    0,
    3,
  );
  const jobList = Object.values(jobs);

  const jobOf = (book: LibraryBook) =>
    book.audio?.transcript_job ? jobs[book.audio.transcript_job] : undefined;

  /**
   * Sofort weiter, nur mit vorhandenem Audio: Hörbuch in der Hörleiste,
   * vertonte Textfassung im Reader mit Autostart. Ohne Audio gibt es kein
   * Abspielen, sondern Lesen.
   */
  const play = (v: BookView) => {
    if (
      v.book.audio_path &&
      (v.resumeWith === "audio" ||
        (v.audioSource === "audiobook" && !v.book.text))
    ) {
      setListening(v.book);
      return;
    }
    if (v.book.text && v.audioSource === "narrated")
      openInReader(v.book.text.page_id, true);
    else if (v.book.audio_path) setListening(v.book);
  };

  const toggleFinished = async (v: BookView) => {
    const finished = !isFinished(v);
    if (v.book.text)
      setFinished(`text:${v.book.text.page_id}:original`, finished);
    if (v.book.audio_path) setFinished(`audio:${v.book.id}`, finished);
    await save(v.book.id, (fresh) => ({ ...fresh, finished }));
  };

  const open = openId ? views.find((v) => v.book.id === openId) : undefined;

  const playButton = (v: BookView, className = "") =>
    !v.audioSource ? (
      v.book.text ? (
        <button
          type="button"
          className={`library-play library-play--read ${className}`}
          onClick={(e) => {
            e.stopPropagation();
            openInReader(v.book.text!.page_id, false);
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
    return v.book.text || v.book.audio_path
      ? t("library.status.notStarted")
      : t("library.status.empty");
  };

  const card = (v: BookView) => (
    <article
      className={`library-card${drop.targetId === v.book.id ? " library-card--drop" : ""}`}
      key={v.book.id}
      data-testid="library-card"
      data-drop-book={v.book.id}
    >
      <div style={{ position: "relative" }}>
        <button
          type="button"
          className="library-card__open"
          onClick={() => setOpenId(v.book.id)}
          aria-label={t("library.openTitle", { title: v.book.title })}
        >
          <BookCover book={v.book} />
        </button>
        {playButton(v, "library-card__play")}
      </div>
      {v.fraction > 0 && !isFinished(v) && (
        <ProgressBar
          fraction={v.fraction}
          label={t("library.progressLabel", { title: v.book.title })}
        />
      )}
      <div className="library-title" lang="de">
        {v.book.title}
      </div>
      <div className="library-meta">{v.book.authors.join(", ")}</div>
      <div className="library-meta">{statusLine(v)}</div>
      <div className="library-card__row">
        <Formats view={v} job={jobOf(v.book)} />
        <span className="library-touch">
          <ActionMenu
            trigger={{
              icon: MoreVertical,
              label: t("library.more", { title: v.book.title }),
              description: t("library.moreHint"),
              testId: `library-more-${v.book.id}`,
            }}
            menuLabel={t("library.moreMenu")}
            align="end"
            items={[
              {
                id: "details",
                label: t("library.details"),
                icon: BookOpen,
                onSelect: () => setOpenId(v.book.id),
              },
              ...(v.book.text
                ? [
                    {
                      id: "read",
                      label: t("library.read"),
                      icon: BookOpen,
                      onSelect: () => openInReader(v.book.text!.page_id, false),
                    },
                  ]
                : []),
              ...(v.book.audio_path
                ? [
                    {
                      id: "listen",
                      label: t("library.listen"),
                      icon: Headphones,
                      onSelect: () => setListening(v.book),
                    },
                  ]
                : []),
              {
                id: "finished",
                label: isFinished(v)
                  ? t("library.markUnfinished")
                  : t("library.markFinished"),
                icon: BookOpen,
                onSelect: () => void toggleFinished(v),
              },
            ]}
          />
        </span>
      </div>
    </article>
  );

  return (
    <PageShell
      title={t("library.title")}
      description={t("library.description")}
      actions={
        <Button
          variant="primary"
          icon={<Plus size={16} aria-hidden="true" />}
          onClick={() => setAdding(true)}
          data-testid="library-add"
        >
          {t("library.add")}
        </Button>
      }
    >
      <div
        className={`library${drop.over ? " library--drop" : ""}`}
        data-library-drop=""
        style={
          { "--lib-bar": listening ? "64px" : "0px" } as React.CSSProperties
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
              onListen={() => setListening(open.book)}
              onToggleFinished={() => void toggleFinished(open)}
              onSave={save}
              onDeleted={async () => {
                setOpenId(null);
                if (listening?.id === open.book.id) setListening(null);
                await reload();
              }}
              onChanged={reload}
            />
          </div>
        ) : (
          <>
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
                {(continueViews.length > 0 || jobList.length > 0) &&
                  filter === "all" &&
                  !query && (
                    <div
                      className={`library-top ${jobList.length ? "" : "library-top--solo"}`}
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
                          <div className="library-continue">
                            {continueViews.map((v) => (
                              <div className="library-ccard" key={v.book.id}>
                                <BookCover book={v.book} size="xs" />
                                <div style={{ minWidth: 0 }}>
                                  <button
                                    type="button"
                                    className="library-ccard__title"
                                    onClick={() => setOpenId(v.book.id)}
                                  >
                                    {v.book.title}
                                  </button>
                                  <div className="library-meta">
                                    {v.book.authors.join(", ")}
                                  </div>
                                  <div style={{ margin: "6px 0 4px" }}>
                                    <ProgressBar
                                      fraction={v.fraction}
                                      label={t("library.progressLabel", {
                                        title: v.book.title,
                                      })}
                                    />
                                  </div>
                                  <div className="library-meta">
                                    {statusLine(v)}
                                  </div>
                                </div>
                                {playButton(v)}
                              </div>
                            ))}
                          </div>
                        </section>
                      )}
                      {jobList.length > 0 && (
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
                                    {t(`library.job.${job.status}`, {
                                      defaultValue: t("library.job.processing"),
                                    })}
                                  </div>
                                </div>
                              </div>
                            );
                          })}
                        </section>
                      )}
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
        {listening && (
          <AudioBar
            key={listening.id}
            book={listening}
            onClose={() => setListening(null)}
          />
        )}
        {dropDialog}
        <AddBookDialog
          open={adding}
          onOpenChange={setAdding}
          onAdded={async (book) => {
            await reload();
            setOpenId(book.id);
          }}
        />
      </div>
    </PageShell>
  );
};

export default LibraryPage;
