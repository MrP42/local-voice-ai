import React from "react";
import { useTranslation } from "react-i18next";
import { Cloud, HardDrive } from "lucide-react";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { useFunctionLlm } from "@/lib/functionLlm";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onStart: () => void;
}

/**
 * Vor der Sprechfassung (Codex-Auflage lva#231): deutlich sagen, dass der
 * Text im Editor ersetzt wird, und wo er verarbeitet wird -- lokal oder bei
 * welchem Cloud-Anbieter, mit welchem Modell (Modell der Funktion Vorlesen).
 */
export const SpeechConfirmDialog: React.FC<Props> = ({
  open,
  onOpenChange,
  onStart,
}) => {
  const { t } = useTranslation();
  const llm = useFunctionLlm("read_aloud");
  const where = !llm.model
    ? t("tts.speechConfirm.none")
    : llm.local
      ? t("tts.speechConfirm.local", { model: llm.model })
      : t("tts.speechConfirm.cloud", {
          provider: llm.connection,
          model: llm.model,
        });
  const Icon = llm.local ? HardDrive : Cloud;
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.speechConfirm.title")}
      closeLabel={t("common.cancel")}
      contentFades={false}
      footer={
        <>
          <Button
            variant="secondary"
            className="lv-touch-row"
            data-testid="speech-cancel"
            onClick={() => onOpenChange(false)}
          >
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            className="lv-touch-row"
            data-testid="speech-start"
            disabled={!llm.model}
            onClick={() => {
              onOpenChange(false);
              onStart();
            }}
          >
            {t("tts.speechConfirm.start")}
          </Button>
        </>
      }
    >
      <div className="space-y-3 text-sm">
        <p data-testid="speech-replace-note">{t("tts.speechConfirm.body")}</p>
        <p
          className="flex items-start gap-2 rounded-md border border-mid-gray/30 px-3 py-2"
          data-testid="speech-where"
          data-local={llm.local || undefined}
        >
          <Icon
            width={16}
            height={16}
            aria-hidden="true"
            className="mt-0.5 shrink-0"
          />
          <span>{where}</span>
        </p>
      </div>
    </Dialog>
  );
};
