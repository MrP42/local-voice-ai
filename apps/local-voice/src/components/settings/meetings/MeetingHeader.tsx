import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  FileText,
  Folder as FolderIcon,
  Info,
  Mic,
  MonitorPlay,
  Upload,
} from "lucide-react";
import type { Meeting, Participant } from "@/bindings";
import {
  isIndeterminate,
  percentOf,
  type LiveProgress,
} from "@/lib/meetingJobs";
import { orderParticipants } from "@/lib/meetingPeople";
import { IconAction } from "../../ui/IconAction";
import { Input } from "../../ui/Input";
import { TabList, type TabListItem } from "../../ui/TabList";
import { TooltipTrigger } from "../../ui/TooltipTrigger";
import { PersonPopover, type PersonRef } from "./people/PersonPopover";

/** Wie viele Avatare die Chipzeile zeigt; der Rest steht hinter "+N". */
const MAX_AVATARS = 4;

const CHIP =
  "inline-flex h-6 shrink-0 items-center gap-1 whitespace-nowrap rounded-full border border-mid-gray/30 px-2 text-xs text-text/80";

/**
 * Status der Besprechung als Chip: Laufzustand der Aufnahme, Fortschritt der
 * Verarbeitung (P8a: Phase, kleiner Balken, Prozent), abgebrochen, fehlerhaft
 * oder fertig. Gleicher Baustein im Detailkopf und im Details-Dialog.
 */
export const StatusChip: React.FC<{
  meeting: Pick<Meeting, "status">;
  progress?: LiveProgress;
}> = ({ meeting, progress }) => {
  const { t } = useTranslation();
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
  participants: Participant[];
  /** Die Projekte, in denen die Besprechung liegt (Namen in Anzeigereihenfolge). */
  projectNames: string[];
  /** Neuer Titel; liefert eine Fehlermeldung oder `null` bei Erfolg. */
  onRename: (title: string) => Promise<string | null>;
  /** Erhöht sich, wenn das Menü "Umbenennen" wählt (oder F2 gedrückt wird). */
  renameNonce: number;
  onOpenDetails: () => void;
  onOpenProjects: () => void;
  onManagePeople: () => void;
  onPersonFilter: (person: PersonRef) => void;
  onPersonAsk: (person: PersonRef) => void;
  tabs: TabListItem<string>[];
  tab: string;
  onTab: (id: string) => void;
  tabsLabel: string;
  /** Schmales Fenster: das Menue mit allen Aktionen sitzt rechts in der Titelzeile. */
  menu?: React.ReactNode;
}

const dateFormatter = (language: string) =>
  new Intl.DateTimeFormat(language, {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });

/**
 * Kompakter Kopf der Besprechung (Arbeitsfläche, Variante B): Titel (per Klick
 * oder F2 umbenennbar) mit Info-Symbol, eine Chipzeile (Status, Quelle, Datum,
 * Dauer, Projekt, Personen) und die Reiter. Zusammen nicht höher als 120 px;
 * alles Weitere steht im Details-Dialog.
 */
export const MeetingHeader: React.FC<MeetingHeaderProps> = ({
  meeting,
  progress,
  participants,
  projectNames,
  onRename,
  renameNonce,
  onOpenDetails,
  onOpenProjects,
  onManagePeople,
  onPersonFilter,
  onPersonAsk,
  tabs,
  tab,
  onTab,
  tabsLabel,
  menu,
}) => {
  const { t, i18n } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(meeting.title);
  const [error, setError] = useState<string | null>(null);
  const titleTip = `${useId()}-tip`;
  const projectTip = `${useId()}-tip`;
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
          : "sourceLive";
  const SourceIcon =
    meeting.source === "import"
      ? Upload
      : meeting.source === "subtitle"
        ? FileText
        : meeting.source === "youtube"
          ? MonitorPlay
          : Mic;
  const sourceLabel = t(`meetings.header.${sourceKey}`);
  const sourceFull = t(`meetings.meta.sourceKind.${meeting.source}`, {
    defaultValue: sourceLabel,
  });

  const when = new Date((meeting.started_at ?? meeting.created_at) * 1000);
  const ordered = orderParticipants(participants);
  const visible = ordered.slice(0, MAX_AVATARS);
  const hidden = ordered.length - visible.length;

  return (
    <div
      data-testid="rec-detail-head"
      className="@container sticky top-0 z-10 -mx-4 bg-background px-4"
    >
      <div className="flex h-9 items-center gap-1">
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
        className="flex h-7 items-center gap-1.5 overflow-hidden py-0.5"
      >
        <StatusChip meeting={meeting} progress={progress} />
        <span className={CHIP} title={sourceFull} data-testid="source-chip">
          <SourceIcon width={12} height={12} aria-hidden="true" />
          <span className="hidden @[34rem]:inline">{sourceLabel}</span>
          <span className="sr-only @[34rem]:hidden">{sourceFull}</span>
        </span>
        <span className={`${CHIP} tabular-nums`} data-testid="date-chip">
          {dateFormatter(i18n.language).format(when)}
        </span>
        {meeting.duration_ms !== null && meeting.status !== "recording" && (
          <span className={`${CHIP} tabular-nums`} data-testid="duration-chip">
            {t("meetings.header.minutes", {
              count: Math.max(1, Math.round(meeting.duration_ms / 60000)),
            })}
          </span>
        )}
        {projectNames.length > 0 && (
          <TooltipTrigger
            tooltipId={projectTip}
            className="inline-flex min-w-0 shrink"
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
              className={`${CHIP} min-w-0 shrink cursor-pointer hover:border-logo-primary focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary`}
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
        {participants.length > 0 && (
          <span
            role="group"
            aria-label={t("meetings.people.chipsLabel")}
            data-testid="participant-chips"
            className="ms-auto flex shrink-0 items-center gap-1 ps-1"
          >
            {visible.map((participant) => (
              <PersonPopover
                key={participant.human_id}
                participant={participant}
                compact
                onFilter={onPersonFilter}
                onAsk={onPersonAsk}
                onManage={onManagePeople}
              />
            ))}
            {hidden > 0 && (
              <button
                type="button"
                onClick={onManagePeople}
                title={t("meetings.header.peopleMoreLabel")}
                aria-label={t("meetings.header.peopleMoreLabel")}
                className="inline-flex h-6 cursor-pointer items-center rounded-full border border-mid-gray/30 px-1.5 text-[10px] text-text/70 hover:border-logo-primary"
              >
                {t("meetings.header.peopleMore", { count: hidden })}
              </button>
            )}
          </span>
        )}
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
