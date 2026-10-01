import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ProjectKind } from "@/bindings";
import { DEFAULT_TEMPLATE_ID } from "@/lib/meetingNotes";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { TemplatePicker } from "../notes/TemplatePicker";

export interface ProjectMinutesChoice {
  templateId: string;
  kind: ProjectKind;
}

interface ProjectMinutesDialogProps {
  open: boolean;
  /** Zahl der gewaehlten Aufnahmen. */
  count: number;
  project: string;
  onClose: () => void;
  onStart: (choice: ProjectMinutesChoice) => void;
}

const KINDS: ProjectKind[] = ["minutes", "summary"];

/**
 * Vor dem Start: Vorlage (wie beim Einzelprotokoll, auch "Automatisch") und Art
 * (Protokoll oder Zusammenfassung). Die Auswahl der Aufnahmen steht schon in der
 * Liste; hier wird nur noch bestaetigt, was daraus entsteht.
 */
export const ProjectMinutesDialog: React.FC<ProjectMinutesDialogProps> = ({
  open,
  count,
  project,
  onClose,
  onStart,
}) => {
  const { t } = useTranslation();
  const [templateId, setTemplateId] = useState<string>(DEFAULT_TEMPLATE_ID);
  const [kind, setKind] = useState<ProjectKind>("minutes");

  // Jeder Start beginnt mit den Standardwerten: die Wahl der letzten Erzeugung
  // soll nicht unbemerkt in die naechste rutschen.
  useEffect(() => {
    if (open) {
      setTemplateId(DEFAULT_TEMPLATE_ID);
      setKind("minutes");
    }
  }, [open]);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
      title={t("meetings.projectMinutes.dialog.title")}
      description={t("meetings.projectMinutes.dialog.body", {
        count,
        project,
      })}
      closeLabel={t("meetings.projectMinutes.dialog.cancel")}
      footer={
        <>
          <Button
            variant="secondary"
            data-testid="pm-dialog-cancel"
            onClick={onClose}
          >
            {t("meetings.projectMinutes.dialog.cancel")}
          </Button>
          <Button
            data-testid="pm-dialog-start"
            onClick={() => onStart({ templateId, kind })}
          >
            {t("meetings.projectMinutes.dialog.start")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="pm-dialog">
        <fieldset className="space-y-1.5">
          <legend className="text-xs text-text/60">
            {t("meetings.projectMinutes.dialog.kind")}
          </legend>
          <div role="radiogroup" className="grid gap-1.5 sm:grid-cols-2">
            {KINDS.map((option) => {
              const checked = kind === option;
              return (
                <label
                  key={option}
                  className={`flex cursor-pointer items-start gap-2 rounded-lg border px-3 py-2 text-sm transition-colors ${
                    checked
                      ? "border-logo-primary bg-logo-primary/10"
                      : "border-mid-gray/30 hover:bg-mid-gray/10"
                  }`}
                >
                  <input
                    type="radio"
                    name="pm-kind"
                    value={option}
                    checked={checked}
                    onChange={() => setKind(option)}
                    data-testid={`pm-kind-${option}`}
                    className="mt-1"
                  />
                  <span>
                    <span className="block font-medium">
                      {t(`meetings.projectMinutes.kind.${option}`)}
                    </span>
                    <span className="block text-xs text-text/60">
                      {t(`meetings.projectMinutes.kind.${option}Hint`)}
                    </span>
                  </span>
                </label>
              );
            })}
          </div>
        </fieldset>
        <div data-testid="pm-dialog-template">
          <TemplatePicker
            value={templateId}
            onChange={setTemplateId}
            allowAuto
            menuPortal
          />
        </div>
      </div>
    </Dialog>
  );
};
