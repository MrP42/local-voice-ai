import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands, type ExportParts } from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import {
  EXPORT_FORMATS,
  EXPORT_PART_KEYS as PART_KEYS,
  exportFileName,
  isPdfFailure,
  type ExportFormat,
} from "@/lib/meetingExport";

const ALL_PARTS: ExportParts = {
  notes: true,
  ai_notes: true,
  minutes: true,
  transcript: true,
  participants: true,
};

interface MeetingExportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  meetingTitle: string;
}

/**
 * M6-P6d: Export einer ganzen Besprechung. Die Haekchen waehlen die Teile,
 * ein Knopf je Format oeffnet den Speichern-Dialog; die Endung entscheidet im
 * Backend (`meetings_export`) ueber das Format. "Formatiert kopieren" legt
 * HTML und Klartext in die Zwischenablage. Audio wird nie exportiert.
 */
export const MeetingExportDialog: React.FC<MeetingExportDialogProps> = ({
  open,
  onOpenChange,
  meetingId,
  meetingTitle,
}) => {
  const { t } = useTranslation();
  const [parts, setParts] = useState<ExportParts>(ALL_PARTS);
  const [busy, setBusy] = useState<ExportFormat | "copy" | null>(null);

  const anyPart = PART_KEYS.some((key) => parts[key]);

  const exportAs = async (format: ExportFormat) => {
    const target = await save({
      filters: [
        {
          name: t(`meetings.export.formats.${format}`),
          extensions: [format],
        },
      ],
      defaultPath: exportFileName(meetingTitle, format),
    });
    if (typeof target !== "string") return;
    setBusy(format);
    const result = await commands.meetingsExport(meetingId, target, parts);
    setBusy(null);
    if (result.status === "ok") {
      toast.success(t("meetings.export.saved", { path: target }));
      return;
    }
    toast.error(
      t("meetings.export.error", { error: result.error }) +
        (isPdfFailure(result.error)
          ? ` ${t("meetings.export.pdfFallback")}`
          : ""),
    );
  };

  const copyFormatted = async () => {
    setBusy("copy");
    const result = await commands.meetingsCopyFormatted(meetingId, parts);
    setBusy(null);
    if (result.status === "ok") {
      toast.success(t("meetings.export.copied"));
      return;
    }
    toast.error(t("meetings.export.copyError", { error: result.error }));
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.export.title")}
      description={t("meetings.export.audioHint")}
      closeLabel={t("meetings.export.close")}
    >
      <div className="space-y-4" data-testid="export-dialog">
        <fieldset className="space-y-1.5">
          <legend className="text-sm font-medium text-text">
            {t("meetings.export.partsTitle")}
          </legend>
          {PART_KEYS.map((key) => (
            <label
              key={key}
              className="flex items-center gap-2 text-sm text-text cursor-pointer"
            >
              <input
                type="checkbox"
                checked={parts[key]}
                onChange={(event) =>
                  setParts((prev) => ({ ...prev, [key]: event.target.checked }))
                }
              />
              {t(`meetings.export.parts.${key}`)}
            </label>
          ))}
          {!anyPart && (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              {t("meetings.export.noParts")}
            </p>
          )}
        </fieldset>

        <div className="space-y-1.5">
          <p className="text-sm font-medium text-text">
            {t("meetings.export.formatsTitle")}
          </p>
          <div className="flex flex-wrap gap-2">
            {EXPORT_FORMATS.map((format) => (
              <Button
                key={format}
                size="sm"
                variant="secondary"
                disabled={!anyPart || busy !== null}
                onClick={() => void exportAs(format)}
                data-testid={`export-${format}`}
              >
                {t(`meetings.export.formats.${format}`)}
              </Button>
            ))}
          </div>
          <p className="text-xs text-text/60">
            {t("meetings.export.subtitleHint")}
          </p>
        </div>

        <div className="border-t border-mid-gray/20 pt-3">
          <Button
            size="sm"
            variant="secondary"
            disabled={!anyPart || busy !== null}
            onClick={() => void copyFormatted()}
            title={t("meetings.export.copyTitle")}
            data-testid="export-copy"
          >
            <Copy width={14} height={14} aria-hidden="true" />
            {t("meetings.export.copy")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};
