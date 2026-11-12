import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";

interface PlaybackCaptureDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Der Start läuft: „Abbrechen“, Escape, Hintergrund und X brechen ihn ab. */
  busy: boolean;
  /** Der Abbruch ist unterwegs (Stopp ist abgeschickt): nichts mehr anklickbar. */
  aborting?: boolean;
  onConfirm: () => void;
  /** Bricht den laufenden Start ab (der Aufruf, der auch den Stopp-Knopf bedient). */
  onAbort: () => void;
}

/**
 * Einwilligung zum Mitschreiben beim Abspielen (#537). Dasselbe Muster wie der
 * Startdialog der Aufnahme (`StartRecordingDialog`): erst mit der Bestätigung
 * unten startet die Aufnahme, ohne sie nichts (das Backend verweigert den
 * Start ohne `consent_confirmed`). Hier gibt es nichts zu wählen, weil Titel,
 * Projekt und Notizen der YouTube-Besprechung bleiben und nur der Systemton
 * aufgenommen wird.
 *
 * Solange der Start läuft (`busy`), ist jeder Weg hinaus ein Abbruch des Starts:
 * „Abbrechen“, Escape, Klick auf den Hintergrund und das X rufen `onAbort`,
 * nicht bloß `onOpenChange(false)`. Sonst schlösse sich der Dialog, und der Start
 * liefe unsichtbar weiter bis zu einer Aufnahme, die niemand mehr wollte (lva#538).
 */
export const PlaybackCaptureDialog: React.FC<PlaybackCaptureDialogProps> = ({
  open,
  onOpenChange,
  busy,
  aborting = false,
  onConfirm,
  onAbort,
}) => {
  const { t } = useTranslation();
  const leave = () => {
    if (!busy) {
      onOpenChange(false);
      return;
    }
    if (!aborting) onAbort();
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) onOpenChange(true);
        else leave();
      }}
      title={t("meetings.variants.youtube.capture.dialog.title")}
      closeLabel={t("meetings.consent.cancel")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={leave}
            disabled={aborting}
            data-testid="yt-capture-cancel"
          >
            {busy
              ? t("meetings.variants.youtube.capture.dialog.abort")
              : t("meetings.consent.cancel")}
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
