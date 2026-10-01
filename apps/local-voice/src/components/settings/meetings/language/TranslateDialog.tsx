import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Languages } from "lucide-react";
import { toast } from "sonner";
import { commands, type TranscriptVariant } from "@/bindings";
import { Alert } from "../../../ui/Alert";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Select } from "../../../ui/Select";
import {
  translateVariantError,
  variantListLabel,
} from "../variants/useVariants";
import {
  appLanguage,
  baseLanguage,
  languageName,
  languageOptions,
} from "./languages";

interface TranslateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  variants: TranscriptVariant[];
  /** Sprache des Transkripts (Chip), falls bekannt. */
  transcriptLanguage: string | null;
  /** Die Übersetzung ist fertig: der Aufrufer lädt die Fassungen neu. */
  onTranslated: (variant: TranscriptVariant) => Promise<void> | void;
  /** "Vergleichen" im Hinweis nach der Übersetzung. */
  onCompare: () => void;
}

/**
 * „Übersetzen nach …“: das lokale Modell übersetzt eine Fassung Satz für Satz in die
 * Zielsprache und legt eine NEUE Fassung an. Das Original bleibt unverändert und jederzeit
 * wählbar (Fassungs-Chip). Die Übersetzung läuft als Auftrag der Besprechung (Fortschritt,
 * Pause, Stopp in der Bedienspalte); nach dem Start schließt der Dialog.
 */
export const TranslateDialog: React.FC<TranslateDialogProps> = ({
  open,
  onOpenChange,
  meetingId,
  variants,
  transcriptLanguage,
  onTranslated,
  onCompare,
}) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const active = variants.find((v) => v.active) ?? variants[0] ?? null;
  const [sourceId, setSourceId] = useState<string>("");
  const [target, setTarget] = useState<string | null>(null);

  const source = variants.find((v) => v.id === sourceId) ?? active;
  const sourceLanguage = baseLanguage(source?.language ?? transcriptLanguage);

  // Vorbelegung beim Öffnen: aktive Fassung als Quelle; Ziel = Sprache der App, ist das
  // die Sprache der Quelle schon, dann Englisch (bzw. Deutsch).
  useEffect(() => {
    if (!open) return;
    setSourceId(active?.id ?? "");
    const app = appLanguage(locale);
    const from = baseLanguage(active?.language ?? transcriptLanguage);
    setTarget(from === app ? (app === "en" ? "de" : "en") : app);
  }, [open, active?.id, active?.language, transcriptLanguage, locale]);

  const sourceOptions = variants.map((v) => ({
    value: v.id,
    label: variantListLabel(v, t),
  }));
  const targetOptions = useMemo(
    () =>
      languageOptions(locale).map((o) => ({
        ...o,
        isDisabled: o.value === sourceLanguage,
      })),
    [locale, sourceLanguage],
  );

  const start = () => {
    if (!target || !source) return;
    onOpenChange(false);
    const language = languageName(target, locale);
    // Der Fortschritt steht danach in der Bedienspalte; Fehler und Ergebnis als Hinweis.
    void commands
      .transcriptVariantTranslate(meetingId, source.id, target)
      .then(async (result) => {
        if (result.status !== "ok") {
          // Ein Stopp ist kein Fehler.
          if (!result.error.startsWith("translate_cancelled")) {
            toast.error(translateVariantError(result.error, t));
          }
          return;
        }
        await onTranslated(result.data);
        toast.success(
          t("meetings.translate.done", {
            number: result.data.number,
            language,
          }),
          {
            action: {
              label: t("meetings.translate.compare"),
              onClick: onCompare,
            },
          },
        );
      });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.translate.title")}
      description={t("meetings.translate.body")}
      closeLabel={t("meetings.translate.close")}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("meetings.translate.cancel")}
          </Button>
          <Button
            data-testid="translate-start"
            disabled={!target || !source}
            onClick={start}
          >
            <Languages width={14} height={14} aria-hidden="true" />
            {t("meetings.translate.start")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="translate-dialog">
        {variants.length > 1 && (
          <div className="space-y-1">
            <span className="text-xs font-medium text-text/60">
              {t("meetings.translate.source")}
            </span>
            <div data-testid="translate-source">
              <Select
                ariaLabel={t("meetings.translate.source")}
                value={source?.id ?? null}
                options={sourceOptions}
                isClearable={false}
                menuPortal
                onChange={(value) => value && setSourceId(value)}
              />
            </div>
          </div>
        )}
        <div className="space-y-1">
          <span className="text-xs font-medium text-text/60">
            {t("meetings.translate.target")}
          </span>
          <div data-testid="translate-target">
            <Select
              ariaLabel={t("meetings.translate.target")}
              value={target}
              options={targetOptions}
              isClearable={false}
              menuPortal
              placeholder={t("meetings.translate.target")}
              onChange={(value) => setTarget(value)}
            />
          </div>
        </div>
        <Alert variant="info">
          <span data-testid="translate-hint">
            {t("meetings.translate.hint")}
          </span>
        </Alert>
      </div>
    </Dialog>
  );
};
