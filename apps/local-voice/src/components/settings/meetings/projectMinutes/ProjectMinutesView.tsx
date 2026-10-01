import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { Check, Copy, Download, FileText, Trash2 } from "lucide-react";
import {
  commands,
  type EntrySource,
  type ProjectMinutes,
  type SourceRecording,
} from "@/bindings";
import type { LiveProgress } from "@/lib/meetingJobs";
import { formatMeetingDate, formatMeetingTimestamp } from "@/lib/meetingDate";
import {
  clock,
  kindTitleKey,
  notifyProjectMinutesChanged,
  recordingOf,
  shortTitle,
} from "@/lib/projectMinutes";
import { Alert } from "../../../ui/Alert";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { IconAction } from "../../../ui/IconAction";
import { MarkdownContent } from "../../../whats-new/MarkdownContent";
import { JobPanel } from "../JobProgress";

/** Was die Arbeitsflaeche zeigt: ein gespeichertes Dokument oder den laufenden Lauf. */
export type OpenProjectMinutes =
  { kind: "doc"; id: string } | { kind: "run"; folderId: string };

interface ProjectMinutesViewProps {
  id: string;
  /** Ein Klick auf einen Beleg: die Aufnahme oeffnen und zur Stelle springen. */
  onOpenSource: (source: EntrySource, doc: ProjectMinutes) => void;
  /** Ein Klick auf eine Quellaufnahme: sie oeffnen (ohne Sprung). */
  onOpenRecording: (recording: SourceRecording) => void;
  /** Das Dokument wurde geloescht (die Ansicht schliesst sich). */
  onDeleted: () => void;
}

/**
 * Ein gespeichertes Projekt-Protokoll in der Arbeitsflaeche: Abschnitte mit
 * Belegen je Aussage (Aufnahme und Zeit, anklickbar), die Quellaufnahmen und die
 * Herkunft. Kopieren und Herunterladen wie beim Einzelprotokoll.
 */
export const ProjectMinutesView: React.FC<ProjectMinutesViewProps> = ({
  id,
  onOpenSource,
  onOpenRecording,
  onDeleted,
}) => {
  const { t, i18n } = useTranslation();
  const [doc, setDoc] = useState<ProjectMinutes | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "gone" | "error">(
    "loading",
  );
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // Die Vorschau dient zugleich als Quelle der formatierten Zwischenablage:
  // was in Word landet, ist genau, was die Markdown-Darstellung zeigt.
  const copyRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setState("loading");
    const result = await commands.projectMinutesGet(id);
    if (result.status !== "ok") {
      setError(String(result.error));
      setState("error");
      return;
    }
    setDoc(result.data ?? null);
    setState(result.data ? "ready" : "gone");
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (state === "loading") {
    return (
      <p className="p-3 text-center text-sm text-text/60">
        {t("meetings.list.loading")}
      </p>
    );
  }
  if (state === "error") {
    return (
      <div className="p-3">
        <Alert variant="error">
          {t("meetings.projectMinutes.view.loadError", { error })}
        </Alert>
      </div>
    );
  }
  if (state === "gone" || !doc) {
    return (
      <p className="p-3 text-sm text-text/60" data-testid="pm-gone">
        {t("meetings.projectMinutes.view.gone")}
      </p>
    );
  }

  const copy = async () => {
    setError(null);
    const html = copyRef.current?.innerHTML;
    try {
      if (html && typeof ClipboardItem !== "undefined") {
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/html": new Blob([html], { type: "text/html" }),
            "text/plain": new Blob([doc.body], { type: "text/plain" }),
          }),
        ]);
      } else {
        await navigator.clipboard.writeText(doc.body);
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      setError(`${t("meetings.projectMinutes.view.copyError")}: ${String(e)}`);
    }
  };

  const download = async () => {
    setError(null);
    setSaved(null);
    const safeName =
      doc.title.replace(/[\\/:*?"<>|]/g, "_").trim() || "projekt-protokoll";
    const target = await save({
      filters: [
        { name: "Word", extensions: ["docx"] },
        { name: "Text", extensions: ["txt"] },
        { name: "Markdown", extensions: ["md"] },
      ],
      defaultPath: `${safeName}.docx`,
    });
    if (typeof target !== "string") return;
    const result = await commands.meetingsExportDocument(target, doc.body);
    if (result.status !== "ok") {
      setError(
        `${t("meetings.projectMinutes.view.exportError")}: ${result.error}`,
      );
      return;
    }
    setSaved(target);
  };

  const remove = async () => {
    setConfirmDelete(false);
    const result = await commands.projectMinutesDelete(doc.id);
    if (result.status !== "ok") {
      setError(String(result.error));
      return;
    }
    notifyProjectMinutesChanged();
    onDeleted();
  };

  const { meta } = doc;
  const sourceTitle = (source: EntrySource) =>
    recordingOf(doc, source)?.title ?? "";

  return (
    <div className="space-y-3 p-3" data-testid="pm-view" data-pm-id={doc.id}>
      <div className="flex flex-wrap items-center gap-2">
        <FileText width={18} height={18} aria-hidden="true" />
        <h2
          className="min-w-0 flex-1 text-base font-semibold"
          data-testid="pm-title"
        >
          {doc.title}
        </h2>
        <div
          role="toolbar"
          aria-label={t("meetings.projectMinutes.view.toolbar")}
          className="flex items-center gap-2"
          data-testid="pm-toolbar"
        >
          <IconAction
            size="sm"
            icon={copied ? Check : Copy}
            label={
              copied
                ? t("meetings.projectMinutes.view.copied")
                : t("meetings.projectMinutes.view.copy")
            }
            description={t("meetings.projectMinutes.view.copyHint")}
            testId="pm-copy"
            onClick={() => void copy()}
          />
          <IconAction
            size="sm"
            icon={Download}
            label={t("meetings.projectMinutes.view.download")}
            description={t("meetings.projectMinutes.view.downloadHint")}
            testId="pm-export"
            onClick={() => void download()}
          />
          <IconAction
            size="sm"
            icon={Trash2}
            label={t("meetings.projectMinutes.view.delete")}
            description={t("meetings.projectMinutes.view.deleteHint")}
            testId="pm-delete"
            onClick={() => setConfirmDelete(true)}
          />
        </div>
      </div>
      <p className="flex flex-wrap items-center gap-x-2 text-xs text-text/60">
        <span data-testid="pm-kind-label">{t(kindTitleKey(doc.kind))}</span>
        <span>·</span>
        <span data-testid="pm-recording-count">
          {t("meetings.projectMinutes.list.meta", {
            date: formatMeetingTimestamp(
              doc.created_at,
              i18n.language,
              "compact",
            ),
            count: doc.recordings.length,
          })}
        </span>
        <span>·</span>
        <span data-testid="pm-template">{meta.template_title}</span>
      </p>

      {error && <Alert variant="error">{error}</Alert>}
      {saved && (
        <p className="break-all text-xs text-text/60" data-testid="pm-saved">
          {t("meetings.projectMinutes.view.exportSaved", { path: saved })}
        </p>
      )}
      {meta.incomplete && (
        <Alert variant="warning">
          <span data-testid="pm-incomplete">
            {t("meetings.projectMinutes.view.incomplete", {
              gaps: meta.gaps.length > 0 ? meta.gaps.join("; ") : "?",
            })}
          </span>
        </Alert>
      )}
      {(meta.dropped_sources > 0 || meta.unsupported_entries > 0) && (
        <p className="text-xs text-text/60" data-testid="pm-proof-note">
          {meta.unsupported_entries > 0 &&
            t("meetings.projectMinutes.view.unsupportedEntries", {
              count: meta.unsupported_entries,
            })}{" "}
          {meta.dropped_sources > 0 &&
            t("meetings.projectMinutes.view.droppedSources", {
              count: meta.dropped_sources,
            })}
        </p>
      )}

      <div
        className="space-y-4 rounded-lg border border-mid-gray/20 p-4"
        data-testid="pm-doc"
      >
        {doc.sections.map((section) => (
          <section
            key={section.id}
            data-testid="pm-section"
            data-section-id={section.id}
            className="space-y-1.5"
          >
            <h3 className="text-sm font-semibold">{section.title}</h3>
            {section.entries.length === 0 ? (
              <p className="text-sm italic text-text/50">
                {t("meetings.projectMinutes.view.empty")}
              </p>
            ) : (
              <ul className="space-y-1.5">
                {section.entries.map((entry, index) => (
                  <li
                    key={index}
                    data-testid="pm-entry"
                    data-unsupported={entry.unsupported ? "true" : undefined}
                    className="text-sm"
                  >
                    <span>{entry.text}</span>
                    {(entry.assignee || entry.due) && (
                      <span className="ms-1 text-text/60">
                        {[
                          entry.assignee
                            ? t("meetings.projectMinutes.view.assignee", {
                                name: entry.assignee,
                              })
                            : null,
                          entry.due
                            ? t("meetings.projectMinutes.view.due", {
                                date: entry.due,
                              })
                            : null,
                        ]
                          .filter(Boolean)
                          .join(", ")}
                      </span>
                    )}
                    {entry.sources.map((source) => {
                      const title = sourceTitle(source);
                      const time = clock(source.start_ms);
                      return (
                        <button
                          key={`${source.meeting_id}:${source.segment_index}`}
                          type="button"
                          data-testid="pm-source"
                          data-source-meeting={source.meeting_id}
                          data-source-segment={source.segment_index}
                          data-source-ms={source.start_ms}
                          title={t("meetings.projectMinutes.view.sourceHint", {
                            title,
                            time,
                          })}
                          aria-label={t(
                            "meetings.projectMinutes.view.sourceHint",
                            { title, time },
                          )}
                          onClick={() => onOpenSource(source, doc)}
                          className="ms-1 inline-flex cursor-pointer items-center gap-1 rounded-full border border-mid-gray/30 px-2 py-0.5 align-baseline text-[11px] text-text/70 transition-colors hover:border-logo-primary hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
                        >
                          <span className="tabular-nums">
                            A{source.recording}
                          </span>
                          <span>{shortTitle(title, 16)}</span>
                          <span className="tabular-nums">{time}</span>
                        </button>
                      );
                    })}
                    {entry.unsupported && (
                      <span
                        data-testid="pm-no-proof"
                        title={t("meetings.projectMinutes.view.noProofHint")}
                        className="ms-1 rounded-full bg-amber-500/20 px-2 text-[11px] text-amber-700 dark:text-amber-400"
                      >
                        {t("meetings.projectMinutes.view.noProof")}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>
        ))}
      </div>

      <section className="space-y-1.5" data-testid="pm-recordings">
        <h3 className="text-sm font-semibold">
          {t("meetings.projectMinutes.view.recordings")}
        </h3>
        <ul className="space-y-1">
          {doc.recordings.map((recording) => (
            <li key={recording.meeting_id} className="text-sm">
              <button
                type="button"
                data-testid="pm-recording"
                data-recording-meeting={recording.meeting_id}
                title={t("meetings.projectMinutes.view.recordingOpen", {
                  title: recording.title,
                })}
                onClick={() => onOpenRecording(recording)}
                className="cursor-pointer rounded text-start hover:text-logo-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
              >
                <span className="me-2 tabular-nums text-text/60">
                  A{recording.index}
                </span>
                {recording.title}
              </button>
              <span className="ms-2 text-xs text-text/60">
                {formatMeetingDate(
                  new Date(recording.started_at * 1000),
                  i18n.language,
                  "compact",
                )}
                {recording.duration_ms !== null &&
                  ` · ${clock(recording.duration_ms)}`}
              </span>
            </li>
          ))}
        </ul>
      </section>

      <details
        data-testid="pm-origin"
        className="rounded-lg border border-mid-gray/20 px-3 py-2"
      >
        <summary className="cursor-pointer text-xs font-medium text-text/70">
          {t("meetings.projectMinutes.view.origin")}
        </summary>
        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-text/60">
            {t("meetings.projectMinutes.view.originModel")}
          </dt>
          <dd data-testid="pm-origin-model">
            {meta.model} ({meta.provider})
          </dd>
          <dt className="text-text/60">
            {t("meetings.projectMinutes.view.originTemplate")}
          </dt>
          <dd data-testid="pm-origin-template">
            {meta.template_title}
            {meta.auto
              ? ` (${t("meetings.projectMinutes.view.originAuto")})`
              : ""}
          </dd>
          <dt className="text-text/60">
            {t("meetings.projectMinutes.view.originCreated")}
          </dt>
          <dd data-testid="pm-origin-created">
            {formatMeetingTimestamp(doc.created_at, i18n.language, "full")}
          </dd>
          <dt className="text-text/60">
            {t("meetings.projectMinutes.view.originMode")}
          </dt>
          <dd data-testid="pm-origin-mode">
            {meta.single_pass
              ? t("meetings.projectMinutes.view.modeSingle")
              : t("meetings.projectMinutes.view.modeBlocks", {
                  count: meta.chunks_total,
                })}
          </dd>
        </dl>
      </details>

      {/* Nur fuer die formatierte Zwischenablage; nicht zu sehen. */}
      <div ref={copyRef} hidden aria-hidden="true">
        <MarkdownContent markdown={doc.body} />
      </div>

      <Dialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("meetings.projectMinutes.view.deleteTitle")}
        closeLabel={t("meetings.projectMinutes.view.deleteCancel")}
        footer={
          <>
            <Button
              variant="secondary"
              data-testid="pm-delete-cancel"
              onClick={() => setConfirmDelete(false)}
            >
              {t("meetings.projectMinutes.view.deleteCancel")}
            </Button>
            <Button
              variant="danger"
              data-testid="pm-delete-confirm"
              onClick={() => void remove()}
            >
              {t("meetings.projectMinutes.view.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.projectMinutes.view.deleteBody")}
        </p>
      </Dialog>
    </div>
  );
};

interface ProjectMinutesRunProps {
  /** Fortschritt aus dem Verzeichnis der Verarbeitungen; `undefined` = Start laeuft. */
  progress: LiveProgress | undefined;
  /** Fehlertext, wenn der Lauf scheiterte (die Ansicht bleibt dann stehen). */
  error: string | null;
  onClose: () => void;
}

/**
 * Der laufende Lauf in der Arbeitsflaeche: Fortschritt mit Laufzeit, Restdauer und
 * Stopp (dieselbe Statusleiste wie bei der Einzelverarbeitung). Das Ergebnis
 * oeffnet sich von selbst, ein Fehler bleibt stehen.
 */
export const ProjectMinutesRun: React.FC<ProjectMinutesRunProps> = ({
  progress,
  error,
  onClose,
}) => {
  const { t } = useTranslation();
  return (
    <div className="space-y-3 p-3" data-testid="pm-run">
      <div className="flex items-center gap-2">
        <FileText width={18} height={18} aria-hidden="true" />
        <h2 className="text-base font-semibold">
          {t("meetings.projectMinutes.run.title")}
        </h2>
      </div>
      {error ? (
        <>
          <Alert variant="error">
            <span data-testid="pm-run-error">{error}</span>
          </Alert>
          <Button
            variant="secondary"
            size="sm"
            data-testid="pm-run-close"
            onClick={onClose}
          >
            {t("meetings.projectMinutes.run.close")}
          </Button>
        </>
      ) : (
        <>
          <p className="text-sm text-text/70">
            {t("meetings.projectMinutes.run.body")}
          </p>
          {progress ? (
            <JobPanel progress={progress} />
          ) : (
            <p className="text-sm text-text/60" data-testid="pm-run-starting">
              {t("meetings.projectMinutes.run.starting")}
            </p>
          )}
        </>
      )}
    </div>
  );
};
