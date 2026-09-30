import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { Meeting } from "@/bindings";
import type { LiveProgress } from "@/lib/meetingJobs";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { StatusChip } from "./MeetingHeader";

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
}

/**
 * Vollinfo zur Besprechung: alles, was der kompakte Kopf nur andeutet (Status,
 * Quelle und Datei, Spuren, Beginn, Dauer, Sprache, Modell, Einwilligung,
 * Löschdatum des Audios, Segmente).
 */
export const MeetingDetailsDialog: React.FC<MeetingDetailsDialogProps> = ({
  open,
  onOpenChange,
  meeting,
  progress,
  segmentCount,
  projectNames,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const { models, loadModels } = useModelStore();

  useEffect(() => {
    if (open && models.length === 0) void loadModels();
  }, [open, models.length, loadModels]);

  const dateTime = (seconds: number) =>
    new Intl.DateTimeFormat(i18n.language, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(seconds * 1000));

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
      key: "status",
      label: t("meetings.meta.status"),
      value: <StatusChip meeting={meeting} progress={progress} />,
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
        <Button onClick={() => onOpenChange(false)}>
          {t("meetings.detailsDialog.close")}
        </Button>
      }
    >
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
    </Dialog>
  );
};
