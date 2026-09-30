import React from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import type { CalEvent, Folder } from "@/bindings";
import { distinctAttendees } from "@/lib/meetingCalendar";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Input } from "../../ui/Input";
import { Select } from "../../ui/Select";
import { MeetingChatNotice } from "./MeetingChatNotice";
import { TemplatePicker } from "./notes/TemplatePicker";

/** Auswahlwert für "Kein Projekt". */
export const NO_PROJECT = "__none__";

interface StartRecordingDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  busy: boolean;
  onConfirm: () => void;
  title: string;
  onTitleChange: (title: string) => void;
  /** Termin, dem die Aufnahme gehört (Kalender), samt Knopf zum Lösen. */
  eventChoice: CalEvent | null;
  onClearEvent: () => void;
  folders: Folder[];
  projectId: string;
  onProjectChange: (id: string) => void;
  templateId: string | null;
  onTemplateChange: (id: string) => void;
  captureSystem: boolean;
  onCaptureSystemChange: (value: boolean) => void;
  diarizeMic: boolean;
  onDiarizeMicChange: (value: boolean) => void;
}

/**
 * Startdialog der Aufnahme: alles, was vor dem Start feststeht (Titel, Projekt,
 * Vorlage, System-Audio, mehrere Personen), und die Einwilligung (§ 201 StGB).
 * Gestartet wird erst mit der Bestätigung unten; die Bedienspalte trägt dafür
 * nur noch einen Knopf.
 */
export const StartRecordingDialog: React.FC<StartRecordingDialogProps> = ({
  open,
  onOpenChange,
  busy,
  onConfirm,
  title,
  onTitleChange,
  eventChoice,
  onClearEvent,
  folders,
  projectId,
  onProjectChange,
  templateId,
  onTemplateChange,
  captureSystem,
  onCaptureSystemChange,
  diarizeMic,
  onDiarizeMicChange,
}) => {
  const { t } = useTranslation();
  const projectOptions = [
    { value: NO_PROJECT, label: t("meetings.startDialog.projectNone") },
    ...folders.map((f) => ({ value: f.id, label: f.name })),
  ];
  const knownProject = folders.some((f) => f.id === projectId)
    ? projectId
    : NO_PROJECT;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.startDialog.title")}
      closeLabel={t("meetings.consent.cancel")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={busy}
          >
            {t("meetings.consent.cancel")}
          </Button>
          <Button onClick={onConfirm} disabled={busy}>
            {t("meetings.consent.confirm")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="start-dialog">
        <div className="space-y-1">
          <label
            htmlFor="start-title"
            className="text-xs font-medium text-text/60"
          >
            {t("meetings.startDialog.titleLabel")}
          </label>
          <Input
            id="start-title"
            type="text"
            value={title}
            onChange={(e) => onTitleChange(e.target.value)}
            placeholder={t("meetings.record.titlePlaceholder")}
            className="w-full"
            data-testid="start-title"
          />
          {eventChoice && (
            <div
              className="flex items-center gap-1.5 pt-1 text-xs text-text/70"
              data-testid="calendar-chip"
              data-event-key={eventChoice.key}
            >
              <span className="rounded-full bg-logo-primary/20 px-2 py-0.5">
                {distinctAttendees(eventChoice) > 0
                  ? t("meetings.calendar.suggestion.chipWithAttendees", {
                      count: distinctAttendees(eventChoice),
                    })
                  : t("meetings.calendar.suggestion.chip")}
              </span>
              <button
                type="button"
                onClick={onClearEvent}
                aria-label={t("meetings.calendar.suggestion.clear")}
                title={t("meetings.calendar.suggestion.clear")}
                className="rounded p-0.5 hover:bg-mid-gray/20 cursor-pointer"
                data-testid="calendar-chip-clear"
              >
                <X width={12} height={12} />
              </button>
            </div>
          )}
        </div>

        <div className="space-y-1" data-testid="start-project">
          <span
            id="start-project-label"
            className="text-xs font-medium text-text/60"
          >
            {t("meetings.startDialog.project")}
          </span>
          <Select
            value={knownProject}
            options={projectOptions}
            isClearable={false}
            disabled={busy}
            menuPortal
            placeholder={t("meetings.startDialog.project")}
            onChange={(id) => onProjectChange(id ?? NO_PROJECT)}
          />
        </div>

        <div data-testid="record-template">
          <TemplatePicker
            value={templateId}
            onChange={onTemplateChange}
            disabled={busy}
            allowAuto
            menuPortal
          />
        </div>

        <div
          className="space-y-1.5"
          role="group"
          aria-label={t("meetings.startDialog.options")}
        >
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={captureSystem}
              onChange={(e) => onCaptureSystemChange(e.target.checked)}
              className="accent-logo-primary"
              data-testid="capture-system"
            />
            {t("meetings.record.captureSystem")}
          </label>
          {captureSystem && (
            <label
              className="flex items-center gap-2 text-sm"
              title={t("meetings.record.diarizeMicHint")}
            >
              <input
                type="checkbox"
                checked={diarizeMic}
                onChange={(e) => onDiarizeMicChange(e.target.checked)}
                className="accent-logo-primary"
                data-testid="diarize-mic"
              />
              {t("meetings.record.diarizeMic")}
            </label>
          )}
        </div>

        <div className="space-y-1">
          <h3 className="text-sm font-semibold">
            {t("meetings.consent.title")}
          </h3>
          <p className="whitespace-pre-wrap text-sm text-text/80">
            {t("meetings.consent.body")}
          </p>
        </div>
        <MeetingChatNotice testId="consent" />
      </div>
    </Dialog>
  );
};
