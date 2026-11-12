import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";

interface PlaybackCaptureDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Der Start läuft: beide Knöpfe gesperrt. */
  busy: boolean;
  onConfirm: () => void;
}

/**
 * Einwilligung zum Mitschreiben beim Abspielen (#537). Dasselbe Muster wie der
 * Startdialog der Aufnahme (`StartRecordingDialog`): erst mit der Bestätigung
 * unten startet die Aufnahme, ohne sie nichts (das Backend verweigert den
 * Start ohne `consent_confirmed`). Hier gibt es nichts zu wählen, weil Titel,
 * Projekt und Notizen der YouTube-Besprechung bleiben und nur der Systemton
 * aufgenommen wird.
 */
export const PlaybackCaptureDialog: React.FC<PlaybackCaptureDialogProps> = ({
  open,
  onOpenChange,
  busy,
  onConfirm,
}) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.variants.youtube.capture.dialog.title")}
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
            onClick={onConfirm}
            disabled={busy}
            data-testid="yt-capture-confirm"
          >
            {t("meetings.variants.youtube.capture.dialog.confirm")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="yt-capture-dialog">
        <p className="text-sm text-text/80">
          {t("meetings.variants.youtube.capture.explain")}
        </p>
        <div className="space-y-1">
          <h3 className="text-sm font-semibold">
            {t("meetings.consent.title")}
          </h3>
          <p className="whitespace-pre-wrap text-sm text-text/80">
            {t("meetings.variants.youtube.capture.dialog.body")}
          </p>
        </div>
      </div>
    </Dialog>
  );
};
