import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { commands, type Meeting } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Select } from "../../ui/Select";
import { translateMeetingError } from "./meetingErrors";

/** Auswahlwert für "Eingestelltes Besprechungsmodell" (Select kennt keinen leeren Wert). */
const CONFIGURED = "__configured__";

interface RetranscribeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meeting: Meeting;
  /** Nach dem Lauf: der Aufrufer lädt seine Segmente neu. */
  onFinished: () => void;
}

/**
 * Neu transkribieren aus der Aufzeichnung, auf Wunsch mit einem anderen Modell:
 * das Modell, das beim Aufnehmen geladen war, ist nicht zwingend das beste.
 * Eine seltene Aktion, deshalb ein Dialog aus dem Menü statt eines Kastens im
 * Kopf. Der Hinweis sagt, was mit dem bisherigen Transkript passiert (B17).
 *
 * Nur sinnvoll mit Audio auf der Platte; das Menü bietet den Eintrag sonst
 * nicht an.
 */
export const RetranscribeDialog: React.FC<RetranscribeDialogProps> = ({
  open,
  onOpenChange,
  meeting,
  onFinished,
}) => {
  const { t } = useTranslation();
  const { models, loadModels } = useModelStore();
  const [modelId, setModelId] = useState(CONFIGURED);

  useEffect(() => {
    if (open && models.length === 0) void loadModels();
  }, [open, models.length, loadModels]);

  const options = [
    { value: CONFIGURED, label: t("meetings.retranscribe.configuredModel") },
    ...models
      .filter((model) => model.is_downloaded)
      .map((model) => ({ value: model.id, label: model.name })),
  ];

  const start = () => {
    const chosen = modelId === CONFIGURED ? null : modelId;
    onOpenChange(false);
    // Der Fortschritt steht danach in der Bedienspalte (P8a); ein Fehler
    // meldet sich als Hinweis.
    void commands.meetingsRetranscribe(meeting.id, chosen).then((result) => {
      if (result.status === "error") {
        toast.error(translateMeetingError(result.error, t));
        return;
      }
      onFinished();
    });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.retranscribeDialog.title")}
      description={meeting.title}
      closeLabel={t("meetings.retranscribeDialog.close")}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("meetings.retranscribeDialog.cancel")}
          </Button>
          <Button onClick={start} data-testid="retranscribe-start">
            <RefreshCw width={14} height={14} aria-hidden="true" />
            {t("meetings.retranscribe.start")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="retranscribe-dialog">
        <p className="text-sm text-text/80">
          {t("meetings.retranscribe.description")}
        </p>
        <div className="space-y-1">
          <span className="text-xs font-medium text-text/60">
            {t("meetings.retranscribeDialog.model")}
          </span>
          <Select
            value={modelId}
            options={options}
            isClearable={false}
            menuPortal
            placeholder={t("meetings.retranscribeDialog.model")}
            onChange={(id) => setModelId(id ?? CONFIGURED)}
          />
        </div>
        <Alert variant="info">
          <span data-testid="retranscribe-hint">
            {t("meetings.retranscribe.runningHint")}
          </span>
        </Alert>
      </div>
    </Dialog>
  );
};
