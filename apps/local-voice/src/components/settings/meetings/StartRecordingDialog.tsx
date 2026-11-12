import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { commands, type CalEvent, type Folder } from "@/bindings";
import { distinctAttendees } from "@/lib/meetingCalendar";
import { noticeIsRequired, noticeSettingsOf } from "@/lib/meetingNotice";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Input } from "../../ui/Input";
import { Select } from "../../ui/Select";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { ParticipantNotice } from "./ParticipantNotice";
import { TemplatePicker } from "./notes/TemplatePicker";

/** Auswahlwert für "Kein Projekt". */
export const NO_PROJECT = "__none__";

interface StartRecordingDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  busy: boolean;
  /**
   * Bestätigt den Start. `noticeToken` ist das Token dieses Dialogs: nur der Start
   * damit sieht den Nachweis, der hier bereitgestellt wurde (G2, #497).
   */
  onConfirm: (noticeToken: string | null) => void;
  title: string;
  onTitleChange: (title: string) => void;
  /** Termin, dem die Aufnahme gehört (Kalender), samt Knopf zum Lösen. */
  eventChoice: CalEvent | null;
  onClearEvent: () => void;
  /**
   * G1 (#70): Titel des leeren Eintrags, in den die Aufnahme geht. Dann entfaellt
   * die Projektwahl (der Eintrag liegt schon in seinen Projekten).
   */
  targetTitle?: string | null;
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
  targetTitle = null,
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
  const { getSetting } = useSettings();
  // G2 (#497): gilt die Pflicht zum Hinweis an Teilnehmende, startet die
  // Aufnahme erst mit einem Nachweis (das Backend prüft es ebenfalls).
  const [noticeCount, setNoticeCount] = useState(0);
  // Jeder geöffnete Dialog bekommt vom Backend ein eigenes Token. Der Nachweis
  // gehört zu ihm: Schließt der Dialog (Abbrechen, Schließen, nach dem Start),
  // verfällt er; ein späterer Dialog beginnt leer.
  const [dialogToken, setDialogToken] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    let issued: string | null = null;
    setNoticeCount(0);
    setDialogToken(null);
    void (async () => {
      try {
        const r = await commands.meetingNoticeDialogOpen();
        if (r.status !== "ok") return;
        if (cancelled) {
          void commands.meetingNoticeDialogClose(r.data);
          return;
        }
        issued = r.data;
        setDialogToken(r.data);
      } catch {
        // Ohne Token startet nur, wer keinen Nachweis braucht.
      }
    })();
    return () => {
      cancelled = true;
      setDialogToken(null);
      if (issued) void commands.meetingNoticeDialogClose(issued);
    };
  }, [open]);
  const noticeBlocks =
    noticeIsRequired(noticeSettingsOf(getSetting("meeting_notice"))) &&
    noticeCount === 0;
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
          <Button
            onClick={() => onConfirm(dialogToken)}
            disabled={busy || noticeBlocks}
            aria-describedby={
              noticeBlocks ? "consent-notice-required-text" : undefined
            }
            data-testid="start-confirm"
          >
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

        {targetTitle !== null ? (
          <p className="text-xs text-text/70" data-testid="start-target">
            {t("meetings.empty.fillsEntry", { title: targetTitle })}
          </p>
        ) : (
          <div className="space-y-1" data-testid="start-project">
            <span
              id="start-project-label"
              className="text-xs font-medium text-text/60"
            >
              {t("meetings.startDialog.project")}
            </span>
            <Select
              ariaLabel={t("meetings.startDialog.project")}
              value={knownProject}
              options={projectOptions}
              isClearable={false}
              disabled={busy}
              menuPortal
              placeholder={t("meetings.startDialog.project")}
              onChange={(id) => onProjectChange(id ?? NO_PROJECT)}
            />
          </div>
        )}

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
          <ToggleSwitch
            checked={captureSystem}
            onChange={onCaptureSystemChange}
            label={t("meetings.record.captureSystem")}
            description={t("meetings.record.captureSystemHint")}
            testId="capture-system"
          />
          {captureSystem && (
            <ToggleSwitch
              checked={diarizeMic}
              onChange={onDiarizeMicChange}
              label={t("meetings.record.diarizeMic")}
              description={t("meetings.record.diarizeMicHint")}
              testId="diarize-mic"
            />
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
        <ParticipantNotice
          meetingId={null}
          dialogToken={dialogToken}
          testId="consent"
          onRecordsChange={setNoticeCount}
        />
      </div>
    </Dialog>
  );
};
