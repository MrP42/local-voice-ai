import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Loader2 } from "lucide-react";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { PREP_STEPS, type PrepStep } from "@/lib/reader/textPrep";

const STORE_KEY = "lva.ui.tts.prepSteps";

function storedSteps(): PrepStep[] {
  try {
    const raw = JSON.parse(localStorage.getItem(STORE_KEY) ?? "null");
    if (Array.isArray(raw)) {
      const steps = PREP_STEPS.filter((s) => raw.includes(s));
      if (steps.length > 0) return steps;
    }
  } catch {
    // Kaputter Eintrag: Standard.
  }
  return [...PREP_STEPS];
}

interface TextPrepDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Gerade laufender Schritt; `null` = bereit. */
  running: PrepStep | null;
  onRun(steps: PrepStep[]): void;
  onCancel(): void;
}

/**
 * „Text aufbereiten“ (lva#783): ein Dialog, drei Schalter, ein Knopf.
 * Die Regeln ohne KI laufen sofort; die beiden KI-Schritte nutzen das
 * aktive Modell. Das Ergebnis ersetzt den Text, „Rückgängig“ bleibt im Toast.
 */
export const TextPrepDialog: React.FC<TextPrepDialogProps> = ({
  open,
  onOpenChange,
  running,
  onRun,
  onCancel,
}) => {
  const { t } = useTranslation();
  const [steps, setSteps] = useState<PrepStep[]>(storedSteps);
  useEffect(() => {
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(steps));
    } catch {
      // Ohne Speicher gilt die Wahl nur für diese Sitzung.
    }
  }, [steps]);

  const busy = running !== null;
  const toggle = (step: PrepStep, on: boolean) =>
    setSteps((prev) =>
      PREP_STEPS.filter((s) => (s === step ? on : prev.includes(s))),
    );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && busy) onCancel();
        onOpenChange(next);
      }}
      title={t("tts.prep.title")}
      description={t("tts.prep.description")}
      closeLabel={t("tts.prep.close")}
      className="w-full max-w-lg"
      contentFades={false}
      footer={
        <div className="flex w-full flex-wrap gap-2">
          <Button
            className="min-h-[44px]"
            variant="primary"
            disabled={busy || steps.length === 0}
            onClick={() => onRun(steps)}
            data-testid="tts-prep-run"
          >
            {busy ? t("tts.prep.running") : t("tts.prep.run")}
          </Button>
          <Button
            className="min-h-[44px]"
            variant="secondary"
            onClick={() => {
              if (busy) onCancel();
              onOpenChange(false);
            }}
            data-testid="tts-prep-cancel"
          >
            {t("common.cancel")}
          </Button>
        </div>
      }
    >
      <ul className="flex flex-col gap-1" data-testid="tts-prep-steps">
        {PREP_STEPS.map((step) => {
          const done =
            busy && PREP_STEPS.indexOf(step) < PREP_STEPS.indexOf(running);
          return (
            <li key={step}>
              <label className="flex min-h-[44px] cursor-pointer items-start gap-3 rounded-md px-2 py-2 hover:bg-mid-gray/10">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={steps.includes(step)}
                  disabled={busy}
                  onChange={(e) => toggle(step, e.target.checked)}
                  data-testid={`tts-prep-${step}`}
                />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="flex items-center gap-2 text-sm font-medium">
                    {t(`tts.prep.steps.${step}`)}
                    {step !== "rules" && (
                      <span className="rounded bg-mid-gray/15 px-1.5 text-[11px] font-normal text-text/70">
                        {t("tts.prep.ai")}
                      </span>
                    )}
                    {running === step && (
                      <Loader2
                        className="h-3.5 w-3.5 animate-spin"
                        aria-label={t("tts.prep.running")}
                      />
                    )}
                    {done && steps.includes(step) && (
                      <Check className="h-3.5 w-3.5 text-green-500" />
                    )}
                  </span>
                  <span className="text-xs text-text/60">
                    {t(`tts.prep.hints.${step}`)}
                  </span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
      <p className="mt-2 px-2 text-xs text-text/60">{t("tts.prep.keeps")}</p>
    </Dialog>
  );
};
