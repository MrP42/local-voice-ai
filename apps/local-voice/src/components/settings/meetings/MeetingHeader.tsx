import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  FileText,
  Folder as FolderIcon,
  Info,
  LayoutTemplate,
  Mic,
  MonitorPlay,
  NotebookPen,
  Upload,
} from "lucide-react";
import type { Meeting } from "@/bindings";
import {
  isIndeterminate,
  percentOf,
  type LiveProgress,
} from "@/lib/meetingJobs";
import { formatMeetingDate } from "@/lib/meetingDate";
import type { QueuePlace } from "@/lib/meetingQueue";
import { IconAction } from "../../ui/IconAction";
import { Input } from "../../ui/Input";
import { TabList, type TabListItem } from "../../ui/TabList";
import { TooltipTrigger } from "../../ui/TooltipTrigger";

const CHIP =
  "inline-flex h-6 shrink-0 items-center gap-1 whitespace-nowrap rounded-full border border-mid-gray/30 px-2 text-xs text-text/80";

/**
 * Status der Besprechung als Chip: Laufzustand der Aufnahme, Fortschritt der
 * Verarbeitung (P8a: Phase, kleiner Balken, Prozent), abgebrochen, fehlerhaft
 * oder fertig. Gleicher Baustein im Detailkopf und im Details-Dialog.
 */
export const StatusChip: React.FC<{
  meeting: Pick<Meeting, "status"> & Partial<Pick<Meeting, "source">>;
  progress?: LiveProgress;
  /** U7: Platz in der Import-Warteschlange (nur bei Status `queued`). */
  queue?: QueuePlace | null;
}> = ({ meeting, progress, queue }) => {
  const { t } = useTranslation();
  if (meeting.status === "queued" && !progress) {
    return (
      <span
        className={`${CHIP} border-mid-gray/40 bg-mid-gray/10`}
        data-testid="status-chip"
        data-state="queued"
        data-position={queue?.position}
        title={
          queue?.reason ? t(`meetings.queue.reason.${queue.reason}`) : undefined
        }
      >
        <span className="truncate">
          {queue
            ? t("meetings.queue.place", {
                position: queue.position,
                total: queue.total,
              })
            : t("meetings.status.queued")}
        </span>
      </span>
    );
  }
  if (progress) {
    const percent = percentOf(progress);
    const indeterminate = isIndeterminate(progress);
    const paused = progress.state === "paused" || progress.state === "pausing";
    return (
      <span
        className={`${CHIP} border-logo-primary/60 bg-logo-primary/10`}
        data-testid="status-chip"
        data-state="processing"
      >
        <span className="truncate">
          {t(`meetings.progress.phase.${progress.phase}`)}
          {paused ? ` · ${t("meetings.progress.state.paused")}` : ""}
        </span>
        <span
          aria-hidden="true"
          className="h-1.5 w-8 shrink-0 overflow-hidden rounded-full bg-mid-gray/30"
        >
          <span
            className={`block h-full rounded-full bg-logo-primary ${
              indeterminate ? "w-full animate-pulse opacity-60" : ""
            }`}
            style={indeterminate ? undefined : { width: `${percent ?? 0}%` }}
          />
        </span>
        {!indeterminate && percent !== null && (
          <span className="tabular-nums">
            {t("meetings.progress.percent", { percent })}
          </span>
        )}
      </span>
    );
  }
  // G1: ein leerer Eintrag ist "fertig" und doch noch nichts: Leer.
  if (meeting.source === "empty" && meeting.status === "ready") {
    return (
      <span
        className={`${CHIP} border-dashed`}
        data-testid="status-chip"
        data-state="empty"
      >
        {t("meetings.empty.chip")}
      </span>
    );
  }
  const label = t(`meetings.status.${meeting.status}`, {
    defaultValue: meeting.status,
  });
  if (meeting.status === "recording") {
    return (
      <span
        className={`${CHIP} border-red-500/50 bg-red-500/10`}
        data-testid="status-chip"
        data-state="recording"
      >
        <span
          aria-hidden="true"
          className="h-2 w-2 animate-pulse rounded-full bg-red-500"
        />
        {label}
      </span>
    );
  }
  if (meeting.status === "ready") {
    return (
      <span
        className={`${CHIP} border-green-500/40 bg-green-500/15 text-emerald-700 dark:text-emerald-400`}
        data-testid="status-chip"
        data-state="ready"
      >
        <Check width={12} height={12} aria-hidden="true" />
        {label}
      </span>
    );
  }
  return (
    <span
      className={`${CHIP} ${
        meeting.status === "failed" ? "border-red-500/50 text-red-500" : ""
      }`}
      data-testid="status-chip"
      data-state={meeting.status}
    >
      {label}
    </span>
  );
};

interface MeetingHeaderProps {
  meeting: Meeting;
  progress?: LiveProgress;
  /** G4: Chip "N Teilnehmende: ..." mit Popover (`ParticipantsPopover`). */
  participantsSlot: React.ReactNode;
  /** G4: Name der gewaehlten Vorlage fuer den Chip "Vorlage: X" (`null` = kein Chip). */
  templateName: string | null;
  /** G4: Klick auf den Vorlagen-Chip (Vorlage wechseln). */
  onOpenTemplate: () => void;
  /** Die Projekte, in denen die Besprechung liegt (Namen in Anzeigereihenfolge). */
  projectNames: string[];
  /** Neuer Titel; liefert eine Fehlermeldung oder `null` bei Erfolg. */
  onRename: (title: string) => Promise<string | null>;
  /** Erhöht sich, wenn das Menü "Umbenennen" wählt (oder F2 gedrückt wird). */
  renameNonce: number;
  /** U7: Platz in der Import-Warteschlange, solange die Besprechung wartet. */
  queue?: QueuePlace | null;
  onOpenDetails: () => void;
  onOpenProjects: () => void;
  tabs: TabListItem<string>[];
  tab: string;
  onTab: (id: string) => void;
  tabsLabel: string;
  /** Das Menue (Hamburger) sitzt in der Titelzeile neben "Details" (schmal mit allen Aktionen). */
  menu?: React.ReactNode;
  /**
   * G1 (#70): der Titel steht beim Anzeigen gleich im Eingabefeld (ein eben
   * angelegter Eintrag). `onAutoEditStarted` meldet, dass es offen ist.
   */
  autoEdit?: boolean;
  onAutoEditStarted?: () => void;
}

/**
 * Kompakter Kopf der Besprechung (Arbeitsfläche, Variante B): Titel (per Klick
 * oder F2 umbenennbar) mit Info-Symbol und Menü, zwei Chipzeilen (Status,
 * Quelle, Datum mit Jahr, Dauer / Projekt, Teilnehmende, Vorlage) und die Reiter
 * Transkript / Protokoll. Zusammen nicht höher als 120 px; alles Weitere steht im
 * Details-Dialog.
 */
export const MeetingHeader: React.FC<MeetingHeaderProps> = ({
  meeting,
  progress,
  queue,
  participantsSlot,
  templateName,
  onOpenTemplate,
  projectNames,
  onRename,
  renameNonce,
  onOpenDetails,
  onOpenProjects,
  tabs,
  tab,
  onTab,
  tabsLabel,
  menu,
  autoEdit = false,
  onAutoEditStarted,
}) => {
  const { t, i18n } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(meeting.title);
  const [error, setError] = useState<string | null>(null);
  const titleTip = `${useId()}-tip`;
  const projectTip = `${useId()}-tip`;
  const templateTip = `${useId()}-tip`;
  const inputRef = useRef<HTMLInputElement>(null);
  // Enter und Escape beenden das Feld, danach löst das Blur kein zweites Speichern aus.
  const finished = useRef(false);

  const startEdit = () => {
    finished.current = false;
    setDraft(meeting.title);
    setError(null);
    setEditing(true);
  };

  // Anfrage von außen (Menü, F2): nicht beim ersten Anzeigen.
  const lastNonce = useRef(renameNonce);
  useEffect(() => {
    if (renameNonce === lastNonce.current) return;
    lastNonce.current = renameNonce;
    startEdit();
  }, [renameNonce]);

  // Ein eben angelegter Eintrag: der Titel ist sofort umbenennbar.
  const autoEditRef = useRef(onAutoEditStarted);
  autoEditRef.current = onAutoEditStarted;
  useEffect(() => {
    if (!autoEdit) return;
    startEdit();
    autoEditRef.current?.();
    // Nur beim Anzeigen mit gesetzter Anfrage.
  }, [autoEdit]);

  useEffect(() => {
    if (!editing) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [editing]);

  const commit = async () => {
    if (finished.current) return;
    finished.current = true;
    const next = draft.trim();
    if (next === "" || next === meeting.title) {
      setEditing(false);
      setError(null);
      return;
    }
    const failure = await onRename(next);
    if (failure) {
      finished.current = false;
      setError(failure);
      return;
    }
    setEditing(false);
    setError(null);
  };

  const cancel = () => {
    finished.current = true;
    setEditing(false);
    setError(null);
  };

  const sourceKey =
    meeting.source === "import"
      ? "sourceImport"
      : meeting.source === "subtitle"
        ? "sourceSubtitle"
        : meeting.source === "youtube"
          ? "sourceYoutube"
          : meeting.source === "empty"
            ? "sourceEmpty"
            : "sourceLive";
  const SourceIcon =
    meeting.source === "import"
      ? Upload
      : meeting.source === "subtitle"
        ? FileText
        : meeting.source === "youtube"
          ? MonitorPlay
          : meeting.source === "empty"
            ? NotebookPen
            : Mic;
  const sourceLabel = t(`meetings.header.${sourceKey}`);
  const sourceFull = t(`meetings.meta.sourceKind.${meeting.source}`, {
    defaultValue: sourceLabel,
  });

  const when = new Date((meeting.started_at ?? meeting.created_at) * 1000);

  return (
    <div
      data-testid="rec-detail-head"
      className="@container sticky top-0 z-10 -mx-4 bg-background px-4"
    >
      <div className="flex h-8 items-center gap-1">
        {editing ? (
          <Input
            ref={inputRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commit();
              if (e.key === "Escape") cancel();
            }}
            onBlur={() => void commit()}
            variant="compact"
            className="min-w-0 flex-1 text-base"
            aria-label={t("meetings.detail.titleLabel")}
            data-testid="meeting-title-input"
          />
        ) : (
          <TooltipTrigger
            tooltipId={titleTip}
            className="flex min-w-0 flex-1"
            content={
              <>
                <div className="text-sm font-semibold">
                  <strong>{t("meetings.header.renameName")}</strong>
                </div>
                <div className="text-xs text-text/70">
                  {t("meetings.header.renameHint")}
                </div>
              </>
            }
          >
            <h3 className="-ms-1 min-w-0 flex-1 text-base font-semibold">
              <button
                type="button"
                data-testid="meeting-title"
                aria-describedby={titleTip}
                onClick={startEdit}
                onKeyDown={(e) => {
                  if (e.key === "F2") {
                    e.preventDefault();
                    startEdit();
                  }
                }}
                className="block w-full cursor-text truncate rounded-md px-1 text-start hover:bg-mid-gray/10 focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary"
              >
                {meeting.title}
              </button>
            </h3>
          </TooltipTrigger>
        )}
        <IconAction
          ghost
          icon={Info}
          label={t("meetings.header.infoName")}
          description={t("meetings.header.infoHint")}
          testId="meeting-details-open"
          onClick={onOpenDetails}
        />
        {menu}
      </div>
      {error && <p className="text-sm text-red-400">{error}</p>}

      <div
        role="group"
        aria-label={t("meetings.header.chips")}
        data-testid="rec-detail-chips"
        className="space-y-0.5 py-0.5"
      >
        <div
          data-testid="rec-detail-chips-main"
          className="flex h-6 items-center gap-1.5 overflow-hidden"
        >
          <StatusChip meeting={meeting} progress={progress} queue={queue} />
          <span className={CHIP} title={sourceFull} data-testid="source-chip">
            <SourceIcon width={12} height={12} aria-hidden="true" />
            <span className="hidden @[34rem]:inline">{sourceLabel}</span>
            <span className="sr-only @[34rem]:hidden">{sourceFull}</span>
          </span>
          <span className={`${CHIP} tabular-nums`} data-testid="date-chip">
            {formatMeetingDate(when, i18n.language, "full")}
          </span>
          {meeting.duration_ms !== null && meeting.status !== "recording" && (
            <span
              className={`${CHIP} tabular-nums`}
              data-testid="duration-chip"
            >
              {t("meetings.header.minutes", {
                count: Math.max(1, Math.round(meeting.duration_ms / 60000)),
              })}
            </span>
          )}
        </div>
        <div
          data-testid="rec-detail-chips-people"
          className="flex h-6 items-center gap-1.5 overflow-hidden"
        >
          {projectNames.length > 0 && (
            <TooltipTrigger
              tooltipId={projectTip}
              className="inline-flex min-w-0 max-w-[45%] shrink-0"
              content={
                <>
                  <div className="text-sm font-semibold">
                    <strong>{projectNames.join(", ")}</strong>
                  </div>
                  <div className="text-xs text-text/70">
                    {t("meetings.header.projectHint")}
                  </div>
                </>
              }
            >
              <button
                type="button"
                onClick={onOpenProjects}
                aria-describedby={projectTip}
                data-testid="project-chip"
                className={`${CHIP} min-w-0 max-w-full cursor-pointer hover:border-logo-primary focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary`}
              >
                <FolderIcon
                  width={12}
                  height={12}
                  aria-hidden="true"
                  className="shrink-0"
                />
                <span className="min-w-0 truncate">{projectNames[0]}</span>
                {projectNames.length > 1 && (
                  <span className="shrink-0 text-text/60">
                    {t("meetings.header.projectMore", {
                      count: projectNames.length - 1,
                    })}
                  </span>
                )}
              </button>
            </TooltipTrigger>
          )}
          {participantsSlot}
          {templateName && (
            <TooltipTrigger
              tooltipId={templateTip}
              className="inline-flex min-w-0 shrink-0 max-w-[40%]"
              content={
                <>
                  <div className="text-sm font-semibold">
                    <strong>
                      {t("meetings.header.template", { name: templateName })}
                    </strong>
                  </div>
                  <div className="text-xs text-text/70">
                    {t("meetings.header.templateHint")}
                  </div>
                </>
              }
            >
              <button
                type="button"
                onClick={onOpenTemplate}
                aria-describedby={templateTip}
                aria-label={t("meetings.header.template", {
                  name: templateName,
                })}
                data-testid="template-chip"
                className={`${CHIP} min-w-0 max-w-full cursor-pointer hover:border-logo-primary focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary`}
              >
                <LayoutTemplate
                  width={12}
                  height={12}
                  aria-hidden="true"
                  className="shrink-0"
                />
                {/* Schmal nur das Symbol (wie bei der Quelle); der Name steht im Tooltip. */}
                <span className="hidden min-w-0 truncate @[34rem]:inline">
                  {t("meetings.header.template", { name: templateName })}
                </span>
              </button>
            </TooltipTrigger>
          )}
        </div>
      </div>

      <TabList
        compact
        tabs={tabs}
        value={tab}
        onChange={onTab}
        ariaLabel={tabsLabel}
        className="border-b border-mid-gray/20"
      />
    </div>
  );
};
