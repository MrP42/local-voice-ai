import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Eye, EyeOff } from "lucide-react";
import { commands, type CalendarSource } from "@/bindings";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Input } from "../../ui/Input";

interface CalendarConnectDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdded: (source: CalendarSource) => void;
}

/**
 * Kalender per ICS-Adresse verbinden (M5-P5b). Der Probeabruf laeuft im
 * Backend VOR dem Speichern: liefert die Adresse keinen lesbaren Kalender,
 * entsteht nichts, und der Grund steht im Dialog. Die Adresse ist ein
 * Geheimnis (Lesezugriff auf den ganzen Kalender) und bleibt deshalb
 * maskiert, bis man sie einblendet; nach dem Speichern zeigt die Oberflaeche
 * nur noch den Host.
 */
export const CalendarConnectDialog: React.FC<CalendarConnectDialogProps> = ({
  open,
  onOpenChange,
  onAdded,
}) => {
  const { t } = useTranslation();
  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("");
  const [reveal, setReveal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) return;
    // Nichts vom Geheimnis bleibt im Zustand, wenn der Dialog zu ist.
    setLabel("");
    setUrl("");
    setReveal(false);
    setBusy(false);
    setError(null);
  }, [open]);

  const connect = async () => {
    if (!url.trim() || busy) return;
    setBusy(true);
    setError(null);
    const result = await commands.calendarSourceAddIcs(
      label.trim(),
      url.trim(),
    );
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    onAdded(result.data);
    onOpenChange(false);
  };

  const toggleLabel = reveal
    ? t("meetings.calendar.dialog.hide")
    : t("meetings.calendar.dialog.show");

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!busy) onOpenChange(next);
      }}
      title={t("meetings.calendar.dialog.title")}
      closeLabel={t("meetings.calendar.cancel")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={busy}
          >
            {t("meetings.calendar.cancel")}
          </Button>
          <Button
            onClick={() => void connect()}
            disabled={busy || !url.trim()}
            data-testid="calendar-connect-submit"
          >
            {busy
              ? t("meetings.calendar.dialog.checking")
              : t("meetings.calendar.dialog.connect")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="calendar-dialog">
        <details className="rounded-lg border border-mid-gray/20 p-3 text-sm">
          <summary className="cursor-pointer font-medium">
            {t("meetings.calendar.dialog.helpTitle")}
          </summary>
          <div className="mt-2 space-y-2 text-text/80">
            <div>
              <p className="font-medium">
                {t("meetings.calendar.dialog.outlookTitle")}
              </p>
              <p>{t("meetings.calendar.dialog.outlookSteps")}</p>
            </div>
            <div>
              <p className="font-medium">
                {t("meetings.calendar.dialog.googleTitle")}
              </p>
              <p>{t("meetings.calendar.dialog.googleSteps")}</p>
            </div>
            <div>
              <p className="font-medium">
                {t("meetings.calendar.dialog.otherTitle")}
              </p>
              <p>{t("meetings.calendar.dialog.otherSteps")}</p>
            </div>
          </div>
        </details>

        <label className="block space-y-1">
          <span className="text-sm font-medium">
            {t("meetings.calendar.dialog.name")}
          </span>
          <Input
            type="text"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder={t("meetings.calendar.dialog.namePlaceholder")}
            disabled={busy}
            className="w-full"
            data-testid="calendar-name"
          />
        </label>

        <div className="space-y-1">
          <label htmlFor="calendar-url" className="text-sm font-medium">
            {t("meetings.calendar.dialog.url")}
          </label>
          <div className="flex items-center gap-2">
            <Input
              id="calendar-url"
              type={reveal ? "text" : "password"}
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder={t("meetings.calendar.dialog.urlPlaceholder")}
              disabled={busy}
              autoComplete="off"
              spellCheck={false}
              className="flex-1"
              data-testid="calendar-url"
            />
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setReveal((v) => !v)}
              aria-label={toggleLabel}
              title={toggleLabel}
              data-testid="calendar-url-toggle"
            >
              {reveal ? (
                <EyeOff width={16} height={16} />
              ) : (
                <Eye width={16} height={16} />
              )}
            </Button>
          </div>
          <p className="text-xs text-text/60">
            {t("meetings.calendar.dialog.privacy")}
          </p>
        </div>

        {error && (
          <div data-testid="calendar-connect-error">
            <Alert variant="error">{error}</Alert>
          </div>
        )}
      </div>
    </Dialog>
  );
};
