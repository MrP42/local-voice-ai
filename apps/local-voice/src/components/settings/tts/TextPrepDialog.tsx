import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Loader2 } from "lucide-react";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { commands, type PrepTarget } from "@/bindings";
import { PREP_STEPS, type PrepStep } from "@/lib/reader/textPrep";

const STORE_KEY = "lva.ui.tts.prepSteps";

function storedSteps(): PrepStep[] {
  try {
    const raw = JSON.parse(localStorage.getItem(STORE_KEY) ?? "null");
    if (Array.isArray(raw)) {
      // Auch eine leere Wahl bleibt, wie sie gespeichert ist.
      return PREP_STEPS.filter((s) => raw.includes(s));
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
  /** `key`: Ziel der KI-Schritte, dem zugestimmt wurde (`tts_prep_target`). */
  onRun(steps: PrepStep[], key: string): void;
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
  // Beide KI-Schritte laufen über das Vorlese-Modell. Ist das ein externer
  // Anbieter, geht nichts hinaus, bevor der Nutzer für genau diesen Lauf
  // zustimmt; das Backend prüft den Schlüssel vor jedem Aufruf erneut und
  // bricht bei einem Modellwechsel ab (lva#783, Codex-Review).
  const [target, setTarget] = useState<PrepTarget | null>(null);
  const [consent, setConsent] = useState(false);
  useEffect(() => {
    if (!open) return;
    setConsent(false);
    setTarget(null);
    let alive = true;
    commands
      .ttsPrepTarget()
      .then((t) => {
        if (alive) setTarget(t ?? null);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [open]);
  const [steps, setSteps] = useState<PrepStep[]>(storedSteps);
  useEffect(() => {
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(steps));
    } catch {
      // Ohne Speicher gilt die Wahl nur für diese Sitzung.
    }
  }, [steps]);

  const busy = running !== null;
  const usesAi = steps.some((s) => s !== "rules");
  // Ohne bekanntes Ziel gilt es als extern: lieber einmal zu viel fragen.
  const remote = usesAi && !(target?.local ?? false);
  const blocked = remote && (!consent || !target);
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
            disabled={busy || steps.length === 0 || blocked}
            onClick={() => onRun(steps, target?.key ?? "")}
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
      {remote && (
        <label
          className="mt-2 flex min-h-[44px] cursor-pointer items-start gap-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-2 text-sm"
          data-testid="tts-prep-cloud"
        >
          <input
            type="checkbox"
            className="mt-1"
            checked={consent}
            disabled={busy || !target}
            onChange={(e) => setConsent(e.target.checked)}
            data-testid="tts-prep-consent"
          />
          <span>
            {target
              ? t("tts.prep.cloud", { provider: target.label || "?" })
              : t("tts.prep.cloudUnknown")}
          </span>
        </label>
      )}
      <p className="mt-2 px-2 text-xs text-text/60">{t("tts.prep.keeps")}</p>
    </Dialog>
  );
};
