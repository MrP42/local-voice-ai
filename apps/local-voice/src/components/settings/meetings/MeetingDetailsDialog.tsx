import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Folder, Meeting, Participant } from "@/bindings";
import type { LiveProgress } from "@/lib/meetingJobs";
import { formatMeetingTimestamp } from "@/lib/meetingDate";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { StatusChip } from "./MeetingHeader";
import { MeetingMetadataForm } from "./MeetingMetadataForm";
import type { QueuePlace } from "@/lib/meetingQueue";

const formatMmSs = (ms: number) => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const ss = seconds.toString().padStart(2, "0");
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${ss}`
    : `${minutes}:${ss}`;
};

// Windows-Pfade haben Backslashes.
const fileBaseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

interface MeetingDetailsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meeting: Meeting;
  progress?: LiveProgress;
  segmentCount: number;
  projectNames: string[];
  /** U7: Platz in der Import-Warteschlange, solange die Besprechung wartet. */
  queue?: QueuePlace | null;
  /** U7 (Bearbeiten): Teilnehmende, alle Projekte, Projekte der Besprechung. */
  participants?: Participant[];
  folders?: Folder[];
  folderIds?: string[];
  /** U7: die Metadaten wurden gespeichert (aktuelle Besprechung). */
  onSaved?: (meeting: Meeting) => void;
  /**
   * G4: Vorlage und Ablage von Protokoll und KI-Notizen. Frueher standen sie im
   * Reiter Protokoll ("Automatisch abgelegt unter ...", "Erzeugt mit der
   * Vorlage ...") und kosteten dort Platz vor dem Inhalt.
   */
  template?: {
    /** Gewaehlte Vorlage (Anzeigename), `null` = Standardvorlage. */
    chosen: string | null;
    /** Vorlage, mit der das Protokoll erzeugt wurde. */
    minutesTemplate: string | null;
    /** Das Modell hat sie nach dem Inhalt gewaehlt. */
    minutesAuto: boolean;
    /** Pfad, unter dem das Protokoll automatisch abgelegt wurde. */
    minutesFile: string | null;
  };
}

/**
 * Vollinfo zur Besprechung: alles, was der kompakte Kopf nur andeutet (Status,
 * Quelle und Datei, Spuren, Beginn, Dauer, Sprache, Modell, Einwilligung,
 * Löschdatum des Audios, Segmente). U7: "Bearbeiten" ändert Titel,
 * Beschreibung, Datum, Teilnehmende und Projekte (`MeetingMetadataForm`);
 * Quelle und Dateiname bleiben, wie sie sind.
 */
export const MeetingDetailsDialog: React.FC<MeetingDetailsDialogProps> = ({
  open,
  onOpenChange,
  meeting,
  progress,
  segmentCount,
  projectNames,
  queue,
  participants = [],
  folders = [],
  folderIds = [],
  onSaved,
  template,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const { models, loadModels } = useModelStore();
  const [editing, setEditing] = useState(false);

  // Ein neu geoeffneter Dialog beginnt immer in der Ansicht.
  useEffect(() => {
    if (!open) setEditing(false);
  }, [open]);

  useEffect(() => {
    if (open && models.length === 0) void loadModels();
  }, [open, models.length, loadModels]);

  const dateTime = (seconds: number) =>
    formatMeetingTimestamp(seconds, i18n.language, "full");

  const tracks = [
    meeting.mic_audio_path
      ? meeting.source === "import"
        ? t("meetings.detailsDialog.trackMixed")
        : t("meetings.detailsDialog.trackMic")
      : null,
    meeting.system_audio_path ? t("meetings.detailsDialog.trackSystem") : null,
  ].filter((x): x is string => x !== null);

  const file = meeting.source_path ?? meeting.mic_audio_path;
  const modelId = getSetting("meeting_model") ?? null;
  const modelName = modelId
    ? (models.find((m) => m.id === modelId)?.name ?? modelId)
    : null;
  let language = t("meetings.detailsDialog.languageAuto");
  if (meeting.language) {
    try {
      language =
        new Intl.DisplayNames([i18n.language], { type: "language" }).of(
          meeting.language,
        ) ?? meeting.language;
    } catch {
      language = meeting.language;
    }
  }

  const rows: { key: string; label: string; value: React.ReactNode }[] = [
    {
      key: "title",
      label: t("meetings.detailsDialog.meetingTitle"),
      value: meeting.title,
    },
    {
      key: "description",
      label: t("meetings.detailsDialog.description"),
      value: meeting.description?.trim() ? (
        <span className="whitespace-pre-wrap break-words">
          {meeting.description}
        </span>
      ) : (
        <span className="text-text/50">
          {t("meetings.detailsDialog.noDescription")}
        </span>
      ),
    },
    {
      key: "status",
      label: t("meetings.meta.status"),
      value: <StatusChip meeting={meeting} progress={progress} queue={queue} />,
    },
    {
      key: "source",
      label: t("meetings.meta.source"),
      value: t(`meetings.meta.sourceKind.${meeting.source}`, {
        defaultValue: meeting.source,
      }),
    },
    ...(file
      ? [
          {
            key: "file",
            label: t("meetings.detailsDialog.file"),
            value: (
              <span className="break-all" title={file}>
                {fileBaseName(file)}
              </span>
            ),
          },
        ]
      : []),
    {
      key: "tracks",
      label: t("meetings.detailsDialog.tracks"),
      value:
        tracks.length > 0
          ? tracks.join(" + ")
          : t("meetings.detailsDialog.trackNone"),
    },
    {
      key: "people",
      label: t("meetings.detailsDialog.people"),
      value:
        participants.length > 0 ? (
          participants.map((p) => p.name).join(", ")
        ) : (
          <span className="text-text/50">
            {t("meetings.detailsDialog.noPeople")}
          </span>
        ),
    },
    {
      key: "projects",
      label: t("meetings.detailsDialog.projects"),
      value:
        projectNames.length > 0
          ? projectNames.join(", ")
          : t("meetings.detailsDialog.noProjects"),
    },
    {
      key: "started",
      label: t("meetings.meta.started"),
      value: dateTime(meeting.started_at ?? meeting.created_at),
    },
    ...(meeting.duration_ms !== null
      ? [
          {
            key: "duration",
            label: t("meetings.meta.duration"),
            value: formatMmSs(meeting.duration_ms),
          },
        ]
      : []),
    {
      key: "language",
      label: t("meetings.detailsDialog.language"),
      value: language,
    },
    {
      key: "model",
      label: t("meetings.detailsDialog.model"),
      value: modelName
        ? t("meetings.detailsDialog.modelNote", { name: modelName })
        : t("meetings.detailsDialog.modelDefault"),
    },
    ...(template
      ? [
          {
            key: "template",
            label: t("meetings.detailsDialog.template"),
            value:
              template.chosen ?? t("meetings.detailsDialog.templateDefault"),
          },
          {
            key: "minutes-origin",
            label: t("meetings.detailsDialog.minutesOrigin"),
            value: template.minutesTemplate ? (
              <span data-testid="minutes-created-with">
                {t(
                  template.minutesAuto
                    ? "meetings.minutes.createdWithAuto"
                    : "meetings.minutes.createdWith",
                  { title: template.minutesTemplate },
                )}
              </span>
            ) : (
              <span className="text-text/50">
                {t("meetings.detailsDialog.minutesNone")}
              </span>
            ),
          },
          {
            key: "minutes-file",
            label: t("meetings.detailsDialog.minutesFile"),
            value: template.minutesFile ? (
              <button
                type="button"
                data-testid="minutes-file-open"
                title={t("meetings.detailsDialog.minutesFileOpen")}
                onClick={() => void revealItemInDir(template.minutesFile!)}
                className="cursor-pointer break-all text-start underline hover:text-logo-primary"
              >
                {template.minutesFile}
              </button>
            ) : (
              <span className="text-text/50">
                {t("meetings.detailsDialog.minutesFileNone")}
              </span>
            ),
          },
        ]
      : []),
    ...(meeting.consent_confirmed_at !== null
      ? [
          {
            key: "consent",
            label: t("meetings.meta.consent"),
            value: dateTime(meeting.consent_confirmed_at),
          },
        ]
      : []),
    {
      key: "retention",
      label: t("meetings.meta.retentionUntil"),
      value:
        meeting.audio_retention_until !== null
          ? dateTime(meeting.audio_retention_until)
          : t("meetings.detailsDialog.retentionNever"),
    },
    {
      key: "segments",
      label: t("meetings.meta.segments"),
      value: segmentCount,
    },
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.detailsDialog.title")}
      closeLabel={t("meetings.detailsDialog.close")}
      footer={
        editing ? undefined : (
          <>
            {onSaved && (
              <Button
                variant="secondary"
                data-testid="details-edit"
                onClick={() => setEditing(true)}
              >
                {t("meetings.metadata.edit")}
              </Button>
            )}
            <Button onClick={() => onOpenChange(false)}>
              {t("meetings.detailsDialog.close")}
            </Button>
          </>
        )
      }
    >
      {editing && onSaved ? (
        <div className="space-y-3">
          {/* Quelle und Dateiname bleiben sichtbar und unveraendert. */}
          <dl
            className="grid grid-cols-[minmax(6rem,auto)_1fr] gap-x-4 gap-y-1.5 text-sm"
            data-testid="meeting-details-fixed"
          >
            <dt className="text-text/60">{t("meetings.meta.source")}</dt>
            <dd data-testid="details-source">
              {t(`meetings.meta.sourceKind.${meeting.source}`, {
                defaultValue: meeting.source,
              })}
            </dd>
            {file && (
              <>
                <dt className="text-text/60">
                  {t("meetings.detailsDialog.file")}
                </dt>
                <dd data-testid="details-file" className="min-w-0 break-all">
                  <span title={file}>{fileBaseName(file)}</span>
                </dd>
              </>
            )}
          </dl>
          <MeetingMetadataForm
            meeting={meeting}
            participants={participants}
            folders={folders}
            folderIds={folderIds}
            onCancel={() => setEditing(false)}
            onSaved={(saved) => {
              setEditing(false);
              onSaved(saved);
            }}
          />
        </div>
      ) : (
        <dl
          className="grid grid-cols-[minmax(6rem,auto)_1fr] gap-x-4 gap-y-1.5 text-sm"
          data-testid="meeting-details"
        >
          {rows.map((row) => (
            <React.Fragment key={row.key}>
              <dt className="text-text/60">{row.label}</dt>
              <dd data-testid={`details-${row.key}`} className="min-w-0">
                {row.value}
              </dd>
            </React.Fragment>
          ))}
        </dl>
      )}
    </Dialog>
  );
};
