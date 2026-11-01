import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  FileDown,
  FileText,
  FileType,
  MoreHorizontal,
  Play,
  Plus,
  Trash2,
} from "lucide-react";
import { commands, type KiDocument } from "@/bindings";
import {
  CHANGED_EVENT,
  docProgressKey,
  removeProgress,
} from "@/lib/library/progress";
import { clearResumePoint } from "@/lib/reader/resumePoint";
import { ActionMenu } from "../../../ui/ActionMenu";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Input } from "../../../ui/Input";
import type { KiDocumentsState } from "./useKiDocuments";
import {
  categoryOf,
  exportKiDocument,
  kiErrorText,
  listenedFraction,
  type KiExportFormat,
  type KiFilter,
} from "./kiDocs";
import "./kidocs.css";

/** Scrollstand der Liste je Seite: bleibt erhalten, während ein Dokument offen ist. */
const scrollMemory = new Map<string, number>();

const FILTERS: KiFilter[] = [
  "all",
  "ueberblick",
  "lernen",
  "analyse",
  "anwenden",
  "import",
];

/**
 * KI-Dokumente eines Buchs als Kartenliste (lva#183, Variante B).
 * Hörfortschritt (Balken) und Erstellung („In Arbeit“) sind getrennt zu sehen;
 * das gerade vorgelesene Dokument trägt ein Abzeichen.
 */
export function KiDocumentsPanel({
  pageId,
  state,
  filter,
  onFilter,
  playingDocId,
  onOpen,
  onPlay,
  onNew,
  onDeleted,
}: {
  pageId: string;
  state: KiDocumentsState;
  filter: KiFilter;
  onFilter: (f: KiFilter) => void;
  playingDocId: string | null;
  onOpen: (doc: KiDocument) => void;
  onPlay: (doc: KiDocument) => void;
  onNew: () => void;
  onDeleted: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<KiDocument | null>(null);
  // Hörfortschritt ändert sich außerhalb (Vorlesen, Gerätesync): neu zeichnen.
  const [, setTick] = useState(0);
  useEffect(() => {
    const bump = () => setTick((n) => n + 1);
    window.addEventListener(CHANGED_EVENT, bump);
    return () => window.removeEventListener(CHANGED_EVENT, bump);
  }, []);

  const listRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    el.scrollTop = scrollMemory.get(pageId) ?? 0;
    // Erst nach dem Laden der Liste hat sie ihre Höhe.
  }, [pageId, state.docs.length]);

  const { docs, templates, run } = state;
  const counts = useMemo(() => {
    const c: Record<string, number> = { all: docs.length };
    for (const d of docs) {
      const k = categoryOf(d, templates);
      c[k] = (c[k] ?? 0) + 1;
    }
    return c;
  }, [docs, templates]);
  const shown = useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    return docs.filter(
      (d) =>
        (filter === "all" || categoryOf(d, templates) === filter) &&
        (!q ||
          d.title.toLocaleLowerCase().includes(q) ||
          d.template_title.toLocaleLowerCase().includes(q) ||
          d.source_label.toLocaleLowerCase().includes(q)),
    );
  }, [docs, templates, filter, query]);

  const doExport = async (doc: KiDocument, format: KiExportFormat) => {
    try {
      if (await exportKiDocument(pageId, doc, format, t))
        toast.success(t("tts.kiDocs.export.done"));
    } catch (e) {
      toast.error(kiErrorText(String((e as Error).message ?? e), t));
    }
  };

  const remove = async (doc: KiDocument) => {
    setConfirmDelete(null);
    const result = await commands.bookAiDocumentDelete(pageId, doc.id);
    if (result.status === "error") {
      toast.error(kiErrorText(result.error, t));
      return;
    }
    // Mit dem Dokument gehen seine Weiterlese-Stelle und sein Fortschritt.
    clearResumePoint(`${pageId}:doc:${doc.id}`);
    removeProgress(docProgressKey(pageId, doc.id));
    onDeleted(doc.id);
    await state.reload();
  };

  const runRatio = run && run.total > 0 ? Math.min(run.done / run.total, 1) : 0;

  return (
    <div
      className="kidocs flex min-h-0 flex-1 flex-col gap-2"
      data-testid="kidocs-panel"
    >
      <div className="flex flex-wrap items-center gap-2">
        <div
          role="group"
          aria-label={t("tts.kiDocs.filterLabel")}
          className="flex flex-wrap gap-1"
        >
          {FILTERS.filter((f) => f === "all" || (counts[f] ?? 0) > 0).map(
            (f) => (
              <button
                key={f}
                type="button"
                aria-pressed={filter === f}
                onClick={() => onFilter(f)}
                data-testid={`kidocs-filter-${f}`}
                className={`kidocs__chip ${filter === f ? "kidocs__chip--on" : ""}`}
              >
                {t(`tts.kiDocs.filter.${f}`)}
                <span className="kidocs__count">{counts[f] ?? 0}</span>
              </button>
            ),
          )}
        </div>
        <div className="kidocs__tools ml-auto flex items-center gap-2">
          <div className="kidocs__searchbox">
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("tts.kiDocs.search")}
              aria-label={t("tts.kiDocs.search")}
              className="kidocs__search"
              data-testid="kidocs-search"
            />
          </div>
          <Button
            variant="primary"
            icon={<Plus className="h-4 w-4" aria-hidden="true" />}
            onClick={onNew}
            disabled={state.busy}
            data-testid="kidocs-new"
          >
            {t("tts.kiDocs.new")}
          </Button>
        </div>
      </div>

      <div
        ref={listRef}
        className="kidocs__grid min-h-0 flex-1 overflow-y-auto"
        onScroll={(e) =>
          scrollMemory.set(pageId, (e.target as HTMLDivElement).scrollTop)
        }
        data-testid="kidocs-list"
      >
        {run && (
          <div
            className="kidocs__card kidocs__card--running"
            data-testid="kidocs-running"
            aria-live="polite"
          >
            <span className="kidocs__meta">
              {run.templateTitle} · {run.sourceLabel}
            </span>
            <span className="kidocs__title">{t("tts.kiDocs.creating")}</span>
            <span className="kidocs__status">
              {run.total > 0
                ? t("tts.kiDocs.creatingSteps", {
                    done: Math.min(run.done, run.total),
                    total: run.total,
                  })
                : t("tts.kiDocs.creatingStart")}
            </span>
            <div
              className="kidocs__bar kidocs__bar--work"
              role="progressbar"
              aria-label={t("tts.kiDocs.creationProgress")}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(runRatio * 100)}
            >
              <div style={{ width: `${runRatio * 100}%` }} />
            </div>
            <div className="kidocs__actions">
              <Button variant="secondary" size="sm" onClick={state.cancel}>
                {t("tts.kiDocs.cancel")}
              </Button>
            </div>
          </div>
        )}
        {shown.map((doc) => {
          const heard = listenedFraction(pageId, doc.id);
          const playing = playingDocId === doc.id;
          return (
            <article
              key={doc.id}
              className={`kidocs__card ${playing ? "kidocs__card--playing" : ""}`}
              data-testid={`kidocs-card-${doc.id}`}
            >
              <button
                type="button"
                className="kidocs__open"
                onClick={() => onOpen(doc)}
                aria-label={t("tts.kiDocs.open", { title: doc.title })}
              >
                <span className="kidocs__meta">
                  {t(`tts.kiDocs.filter.${categoryOf(doc, templates)}`)} ·{" "}
                  {doc.source_label}
                </span>
                <span className="kidocs__title">{doc.title}</span>
                <span className="kidocs__status">
                  {playing && (
                    <span
                      className="kidocs__badge"
                      data-testid="kidocs-playing"
                    >
                      {t("tts.kiDocs.playing")}
                    </span>
                  )}
                  {t("tts.kiDocs.words", { count: doc.words })} ·{" "}
                  {heard >= 0.995
                    ? t("tts.kiDocs.heardAll")
                    : heard > 0
                      ? t("tts.kiDocs.heard", {
                          percent: Math.round(heard * 100),
                        })
                      : t("tts.kiDocs.notHeard")}
                </span>
              </button>
              <div
                className="kidocs__bar"
                role="progressbar"
                aria-label={t("tts.kiDocs.listenProgress")}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(heard * 100)}
              >
                <div style={{ width: `${heard * 100}%` }} />
              </div>
              <div className="kidocs__actions">
                <Button
                  variant="secondary"
                  size="sm"
                  icon={<Play className="h-4 w-4" aria-hidden="true" />}
                  onClick={() => onPlay(doc)}
                  data-testid={`kidocs-play-${doc.id}`}
                >
                  {heard > 0 && heard < 0.995
                    ? t("tts.kiDocs.resume")
                    : t("tts.kiDocs.listen")}
                </Button>
                <ActionMenu
                  trigger={{
                    icon: MoreHorizontal,
                    label: t("tts.kiDocs.more"),
                    testId: `kidocs-more-${doc.id}`,
                    description: t("tts.kiDocs.moreHint"),
                  }}
                  menuLabel={t("tts.kiDocs.more")}
                  align="end"
                  items={[
                    {
                      id: "md",
                      label: t("tts.kiDocs.export.md"),
                      icon: FileText,
                      onSelect: () => void doExport(doc, "md"),
                    },
                    {
                      id: "docx",
                      label: t("tts.kiDocs.export.docx"),
                      icon: FileType,
                      onSelect: () => void doExport(doc, "docx"),
                    },
                    {
                      id: "pdf",
                      label: t("tts.kiDocs.export.pdf"),
                      icon: FileDown,
                      onSelect: () => void doExport(doc, "pdf"),
                    },
                    {
                      id: "delete",
                      label: t("tts.kiDocs.delete"),
                      icon: Trash2,
                      onSelect: () => setConfirmDelete(doc),
                      testId: `kidocs-delete-${doc.id}`,
                    },
                  ]}
                />
              </div>
            </article>
          );
        })}
        {!run && shown.length === 0 && (
          <div className="kidocs__empty" data-testid="kidocs-empty">
            <p>
              {docs.length === 0
                ? t("tts.kiDocs.empty")
                : t("tts.kiDocs.noMatch")}
            </p>
            {docs.length === 0 && (
              <Button
                variant="primary"
                icon={<Plus className="h-4 w-4" aria-hidden="true" />}
                onClick={onNew}
                disabled={state.busy}
              >
                {t("tts.kiDocs.new")}
              </Button>
            )}
          </div>
        )}
      </div>

      <Dialog
        open={confirmDelete !== null}
        onOpenChange={(o) => !o && setConfirmDelete(null)}
        title={t("tts.kiDocs.deleteTitle")}
        closeLabel={t("tts.kiDocs.cancel")}
        footer={
          <>
            <Button
              variant="danger"
              onClick={() => confirmDelete && void remove(confirmDelete)}
              data-testid="kidocs-delete-confirm"
            >
              {t("tts.kiDocs.delete")}
            </Button>
            <Button variant="secondary" onClick={() => setConfirmDelete(null)}>
              {t("tts.kiDocs.cancel")}
            </Button>
          </>
        }
      >
        <p className="text-sm">
          {t("tts.kiDocs.deleteText", { title: confirmDelete?.title ?? "" })}
        </p>
      </Dialog>
    </div>
  );
}
