import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands, type ExportParts, type IntegrationView } from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import {
  EXPORT_FORMATS,
  EXPORT_PART_KEYS as PART_KEYS,
  exportFileName,
  isPdfFailure,
  type ExportFormat,
} from "@/lib/meetingExport";

/** Formate, die sich in einen Ordner ablegen lassen (A6, „ablegen in“). */
const PLACE_FORMATS = ["md", "docx", "pdf"] as const;

/** Ordner und Vaults, in die ein Export abgelegt werden kann: eingeschaltet und schreibend. */
const placeTargets = (views: IntegrationView[]) =>
  views.filter(
    (v) =>
      v.integration.enabled &&
      v.integration.direction !== "read" &&
      (v.integration.kind === "folder" || v.integration.kind === "obsidian"),
  );

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
  const [busy, setBusy] = useState<ExportFormat | "copy" | "place" | null>(
    null,
  );
  const [targets, setTargets] = useState<IntegrationView[]>([]);

  useEffect(() => {
    if (!open) return;
    let alive = true;
    void (async () => {
      try {
        const result = await commands.integrationsList();
        if (alive && result.status === "ok") {
          setTargets(placeTargets(result.data ?? []));
        }
      } catch {
        /* ohne Register gibt es keinen Abschnitt „Ablegen in“ */
      }
    })();
    return () => {
      alive = false;
    };
  }, [open]);

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

  const placeError = (raw: string) => {
    const text = t(`integrations.errors.${raw}`, { defaultValue: "" });
    return t("meetings.export.placeError", { error: text || raw });
  };

  const placeInFolder = async (
    target: IntegrationView,
    format: (typeof PLACE_FORMATS)[number],
  ) => {
    setBusy("place");
    try {
      const result = await commands.integrationExportToFolder(
        target.integration.id,
        meetingId,
        format,
        parts,
      );
      if (result.status === "ok") {
        toast.success(
          t("meetings.export.placed", {
            label: target.integration.label,
            path: result.data.rel,
          }),
        );
      } else {
        toast.error(placeError(result.error));
      }
    } catch (e) {
      toast.error(placeError(String(e)));
    }
    setBusy(null);
  };

  const placeInVault = async (target: IntegrationView) => {
    setBusy("place");
    try {
      const result = await commands.integrationSaveToVault(
        target.integration.id,
        meetingId,
        parts,
      );
      if (result.status === "ok") {
        const key =
          result.data.result === "created"
            ? "vaultCreated"
            : result.data.result === "updated"
              ? "vaultUpdated"
              : "vaultUnchanged";
        toast.success(
          t(`meetings.export.${key}`, {
            label: target.integration.label,
            path: result.data.rel,
          }),
        );
      } else {
        toast.error(placeError(result.error));
      }
    } catch (e) {
      toast.error(placeError(String(e)));
    }
    setBusy(null);
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

        {targets.length > 0 && (
          <div className="space-y-1.5" data-testid="export-place">
            <p className="text-sm font-medium text-text">
              {t("meetings.export.placeTitle")}
            </p>
            <ul className="space-y-1.5">
              {targets.map((target) => (
                <li
                  key={target.integration.id}
                  className="flex flex-wrap items-center gap-2"
                  data-testid="export-place-target"
                  data-integration-id={target.integration.id}
                >
                  <span className="min-w-0 break-words text-sm text-text">
                    {target.integration.kind === "folder"
                      ? t("meetings.export.placeFolder", {
                          label: target.integration.label,
                        })
                      : t("meetings.export.placeVault", {
                          label: target.integration.label,
                        })}
                  </span>
                  {target.integration.kind === "folder" ? (
                    PLACE_FORMATS.map((format) => (
                      <Button
                        key={format}
                        size="sm"
                        variant="secondary"
                        disabled={!anyPart || busy !== null}
                        onClick={() => void placeInFolder(target, format)}
                        data-testid={`place-${target.integration.id}-${format}`}
                      >
                        {t(`meetings.export.formats.${format}`)}
                      </Button>
                    ))
                  ) : (
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={!anyPart || busy !== null}
                      onClick={() => void placeInVault(target)}
                      data-testid={`place-${target.integration.id}-vault`}
                    >
                      {t("meetings.export.placeVaultButton")}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
            <p className="text-xs text-text/60">
              {t("meetings.export.placeHint")}
            </p>
          </div>
        )}

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
