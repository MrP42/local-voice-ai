import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Languages } from "lucide-react";
import { toast } from "sonner";
import { commands, type LanguageInfo } from "@/bindings";
import { Alert } from "../../../ui/Alert";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Select } from "../../../ui/Select";
import { translateMeetingError } from "../meetingErrors";
import { languageName, languageOptions } from "./languages";

const CHIP =
  "inline-flex h-6 shrink-0 items-center gap-1 whitespace-nowrap rounded-full border border-mid-gray/30 px-2 text-xs text-text/80";

export interface LanguageRetranscribeRequest {
  /** Die gewählte Sprache. */
  language: string;
  /** Ein installiertes Modell, das die App für sie empfiehlt (sonst `null`: die App wählt). */
  modelId: string | null;
}

interface LanguageChipProps {
  meetingId: string;
  info: LanguageInfo | null;
  /** Der Chip hat die Angaben geändert (Sprache gesetzt). */
  onInfo: (info: LanguageInfo) => void;
  /** Gibt es ein Transkript? Ohne gibt es keine Sprache anzuzeigen. */
  hasTranscript: boolean;
  /** Liegt Audio auf der Platte? Nur dann lässt sich neu transkribieren. */
  hasAudio: boolean;
  /** Läuft eine Verarbeitung? Dann lässt sich die Sprache nicht ändern. */
  busy: boolean;
  /** "Speichern und neu transkribieren": der Aufrufer öffnet den Dialog der Neu-Transkription. */
  onRetranscribe: (request: LanguageRetranscribeRequest) => void;
  /** "Übersetzen nach …": der Aufrufer öffnet den Dialog der Übersetzung. */
  onTranslate: () => void;
}

/**
 * Chip im Kopf der Besprechung: die Sprache des Transkripts. Ein Klick öffnet den Dialog zum
 * Korrigieren. Die Herkunft steht dabei (Spracherkennung des Modells, am Text geschätzt, von
 * Hand gewählt, aus der Einstellung); deckt das Modell die Sprache nicht ab oder widerspricht
 * der Text der Wahl, steht das dort, mit dem besseren Modell. Korrigieren ändert nur die
 * Angabe; das Transkript ändert die Neu-Transkription mit dem passenden Modell, und die legt
 * eine NEUE Fassung an.
 */
export const LanguageChip: React.FC<LanguageChipProps> = ({
  meetingId,
  info,
  onInfo,
  hasTranscript,
  hasAudio,
  busy,
  onRetranscribe,
  onTranslate,
}) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [open, setOpen] = useState(false);
  const [choice, setChoice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setChoice(info?.code ?? null);
      setError(null);
    }
  }, [open, info?.code]);

  // Ohne Transkript oder solange die Angaben fehlen gibt es nichts anzuzeigen.
  if (!hasTranscript || !info) return null;

  const code = info?.code ?? null;
  const name = code ? languageName(code, locale) : null;
  const warn = Boolean(
    info && (info.mismatch || info.forced || info.suggestion),
  );
  const options = languageOptions(locale);

  const save = async (andRetranscribe: boolean) => {
    if (!choice) return;
    setSaving(true);
    setError(null);
    const result = await commands.meetingsSetLanguage(meetingId, choice);
    setSaving(false);
    if (result.status !== "ok") {
      setError(translateMeetingError(result.error, t));
      return;
    }
    onInfo(result.data);
    setOpen(false);
    toast.success(
      t("meetings.langChip.saved", { language: languageName(choice, locale) }),
    );
    if (andRetranscribe) {
      const suggestion = result.data.suggestion;
      onRetranscribe({
        language: choice,
        modelId: suggestion?.downloaded ? suggestion.model_id : null,
      });
    }
  };

  const sourceText = info?.source
    ? t(`meetings.langChip.source.${info.source}`)
    : t("meetings.langChip.source.none");

  return (
    <>
      <button
        type="button"
        data-testid="language-chip"
        data-language={code ?? ""}
        data-source={info?.source ?? ""}
        aria-haspopup="dialog"
        aria-label={
          name
            ? t("meetings.langChip.chipLabel", { name })
            : t("meetings.langChip.chipUnknown")
        }
        title={sourceText}
        onClick={() => setOpen(true)}
        className={`${CHIP} cursor-pointer hover:border-logo-primary focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary ${
          warn ? "border-yellow-500/60" : ""
        }`}
      >
        <Languages
          width={12}
          height={12}
          aria-hidden="true"
          className="shrink-0"
        />
        {/* Schmal nur der Code; der volle Name steht im Namen für Hilfsmittel. */}
        <span aria-hidden="true" className="@[34rem]:hidden uppercase">
          {code ?? "?"}
        </span>
        <span aria-hidden="true" className="hidden @[34rem]:inline">
          {name ?? t("meetings.langChip.unknownShort")}
        </span>
        {warn && (
          <AlertTriangle
            width={12}
            height={12}
            aria-hidden="true"
            className="shrink-0 text-yellow-500"
          />
        )}
      </button>

      <Dialog
        open={open}
        onOpenChange={setOpen}
        title={t("meetings.langChip.title")}
        description={t("meetings.langChip.body")}
        closeLabel={t("meetings.langChip.close")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setOpen(false)}>
              {t("meetings.langChip.cancel")}
            </Button>
            <Button
              variant="secondary"
              data-testid="language-save"
              disabled={!choice || saving || busy}
              onClick={() => void save(false)}
            >
              {t("meetings.langChip.save")}
            </Button>
            <Button
              data-testid="language-save-retranscribe"
              disabled={!choice || saving || busy || !hasAudio}
              title={
                hasAudio ? undefined : t("meetings.actions.retranscribeNoAudio")
              }
              onClick={() => void save(true)}
            >
              {t("meetings.langChip.saveRetranscribe")}
            </Button>
          </>
        }
      >
        <div className="space-y-3" data-testid="language-dialog">
          <div className="space-y-1">
            <span className="text-xs font-medium text-text/60">
              {t("meetings.langChip.select")}
            </span>
            <div data-testid="language-select">
              <Select
                ariaLabel={t("meetings.langChip.select")}
                value={choice}
                options={options}
                isClearable={false}
                menuPortal
                placeholder={t("meetings.langChip.select")}
                onChange={(value) => setChoice(value)}
              />
            </div>
          </div>
          <p className="text-xs text-text/70" data-testid="language-source">
            {sourceText}
            {info?.model_name
              ? ` · ${t("meetings.langChip.model", { model: info.model_name })}`
              : ""}
          </p>
          {info?.forced && code && (
            <Alert variant="warning">
              <span data-testid="language-forced">
                {t("meetings.langChip.forcedHint", {
                  forced: languageName(info.forced, locale),
                  language: languageName(code, locale),
                })}
              </span>
            </Alert>
          )}
          {info?.mismatch && code && (
            <Alert variant="warning">
              <span data-testid="language-mismatch">
                {t("meetings.langChip.mismatchHint", {
                  language: languageName(info.mismatch, locale),
                  chosen: languageName(code, locale),
                })}
              </span>
            </Alert>
          )}
          {info?.model_covers === false && code && (
            <Alert variant="info">
              <span data-testid="language-suggestion">
                {t("meetings.langChip.notCovered", {
                  model: info.model_name ?? info.model_id ?? "",
                  language: languageName(code, locale),
                })}{" "}
                {info.suggestion
                  ? info.suggestion.downloaded
                    ? t("meetings.langChip.suggestInstalled", {
                        model: info.suggestion.name,
                      })
                    : t("meetings.langChip.suggestMissing", {
                        model: info.suggestion.name,
                      })
                  : ""}
              </span>
            </Alert>
          )}
          {error && <Alert variant="error">{error}</Alert>}
          <div>
            <Button
              size="sm"
              variant="secondary"
              data-testid="language-translate"
              onClick={() => {
                setOpen(false);
                onTranslate();
              }}
            >
              {t("meetings.langChip.translate")}
            </Button>
          </div>
        </div>
      </Dialog>
    </>
  );
};
