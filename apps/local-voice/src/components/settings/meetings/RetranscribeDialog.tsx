import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Radio, RadioGroup } from "@fluentui/react-components";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { commands, type Meeting, type ModelSuggestion } from "@/bindings";
import {
  coversAll,
  fullRange,
  recordingDurationMs,
  type MsRange,
} from "@/lib/meetingRange";
import { useModelStore } from "@/stores/modelStore";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Select } from "../../ui/Select";
import {
  AudioRangePicker,
  type AudioRangePickerHandle,
} from "./AudioRangePicker";
import { baseLanguage, languageOptions } from "./language/languages";
import { translateMeetingError } from "./meetingErrors";

/** Auswahlwert für "Eingestelltes Besprechungsmodell" (Select kennt keinen leeren Wert). */
const CONFIGURED = "__configured__";
/** Auswahlwert für "Sprache automatisch erkennen". */
const AUTO = "auto";
/** lva#522: ganze Aufnahme oder nur ein Bereich. */
type Scope = "all" | "range";

interface RetranscribeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meeting: Meeting;
  /** Nach dem Lauf: der Aufrufer lädt seine Segmente neu. */
  onFinished: () => void;
  /**
   * G5: Vorbelegung, wenn der Dialog aus dem Sprach-Chip kommt: die gewählte Sprache und
   * das empfohlene, schon installierte Modell (`null`: die App wählt es).
   */
  initial?: { language: string; modelId: string | null } | null;
}

/**
 * Neu transkribieren aus der Aufzeichnung, auf Wunsch mit einem anderen Modell und einer
 * festen Sprache: das Modell, das beim Aufnehmen geladen war, ist nicht zwingend das beste,
 * und eine andere Sprache braucht ein Modell, das sie kann. Eine seltene Aktion, deshalb ein
 * Dialog aus dem Menü statt eines Kastens im Kopf. Der Hinweis sagt, was mit dem bisherigen
 * Transkript passiert (B17: es bleibt als Fassung).
 *
 * G5: "Automatisch erkennen" (Standard) bestimmt die Sprache aus dem Audio; eine gewählte
 * Sprache gilt vor jeder Erkennung. Wählt man eine Sprache, schlägt der Dialog das passende
 * Modell vor (installiert: vorbelegt; sonst ein Hinweis). Ein von Hand gewähltes Modell hat
 * immer Vorrang.
 *
 * lva#522: "Nur einen Bereich" transkribiert nur diesen Abschnitt der Aufnahme (Von/Bis und
 * Zweifach-Regler); die Zeitstempel bleiben die der ganzen Aufnahme, das bisherige
 * Transkript bleibt als Fassung. Gedacht für lange Aufnahmen, von denen nur ein Teil zählt.
 *
 * Nur sinnvoll mit Audio auf der Platte; das Menü bietet den Eintrag sonst nicht an.
 */
export const RetranscribeDialog: React.FC<RetranscribeDialogProps> = ({
  open,
  onOpenChange,
  meeting,
  onFinished,
  initial = null,
}) => {
  const { t, i18n } = useTranslation();
  const { models, loadModels } = useModelStore();
  const [modelId, setModelId] = useState(CONFIGURED);
  const [language, setLanguage] = useState<string>(AUTO);
  const [suggestion, setSuggestion] = useState<ModelSuggestion | null>(null);
  // lva#522: Bereich. Ohne bekannte Dauer gibt es keine Auswahl (ganze Aufnahme).
  const durationMs = recordingDurationMs(meeting.duration_ms);
  const [scope, setScope] = useState<Scope>("all");
  const [range, setRange] = useState<MsRange>(fullRange(durationMs ?? 0));
  const pickerRef = useRef<AudioRangePickerHandle>(null);
  const [fieldsValid, setFieldsValid] = useState(true);
  // Hat der Nutzer das Modell selbst gewählt? Dann überschreibt kein Vorschlag es mehr.
  const modelTouched = useRef(false);

  useEffect(() => {
    if (open && models.length === 0) void loadModels();
  }, [open, models.length, loadModels]);

  // Vorbelegung beim Öffnen.
  useEffect(() => {
    if (!open) return;
    modelTouched.current = initial?.modelId != null;
    setLanguage(initial?.language ?? AUTO);
    setModelId(initial?.modelId ?? CONFIGURED);
    setSuggestion(null);
    setScope("all");
    setRange(fullRange(durationMs ?? 0));
  }, [open, initial?.language, initial?.modelId, durationMs]);

  // Das passende Modell der gewählten Sprache (Eignung, Katalog, installiert ja/nein).
  useEffect(() => {
    if (!open || language === AUTO) {
      setSuggestion(null);
      return;
    }
    let current = true;
    void (async () => {
      const info = await commands.meetingsLanguageInfo(meeting.id);
      const currentModel = info.status === "ok" ? info.data.model_id : null;
      const result = await commands.meetingsModelForLanguage(
        currentModel,
        language,
      );
      if (!current) return;
      const found = result.status === "ok" ? (result.data ?? null) : null;
      setSuggestion(found);
      if (found?.downloaded && !modelTouched.current)
        setModelId(found.model_id);
    })();
    return () => {
      current = false;
    };
  }, [open, language, meeting.id]);

  const options = [
    { value: CONFIGURED, label: t("meetings.retranscribe.configuredModel") },
    ...models
      .filter((model) => model.is_downloaded)
      .map((model) => ({ value: model.id, label: model.name })),
  ];
  const languageChoices = useMemo(
    () => [
      { value: AUTO, label: t("meetings.retranscribe.languageAuto") },
      ...languageOptions(i18n.language),
    ],
    [i18n.language, t],
  );

  const start = () => {
    const chosen = modelId === CONFIGURED ? null : modelId;
    const fixed = baseLanguage(language);
    // Die Feldtexte im Moment des Klicks neu lesen: ein unlesbarer Text sperrt, es
    // gibt keinen Rückgriff auf einen älteren Wert.
    let current: MsRange = range;
    if (scope === "range") {
      const read = pickerRef.current?.validate() ?? null;
      if (!read) return;
      current = read;
    }
    // Ein Bereich, der die ganze Aufnahme deckt, ist keiner.
    const chosenRange =
      scope === "range" &&
      durationMs !== null &&
      !coversAll(current, durationMs)
        ? current
        : null;
    onOpenChange(false);
    // Der Fortschritt steht danach in der Bedienspalte (P8a); ein Fehler
    // meldet sich als Hinweis.
    void commands
      .meetingsRetranscribe(meeting.id, chosen, fixed, chosenRange)
      .then((result) => {
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
          <Button
            variant="secondary"
            className="max-sm:min-h-[44px]"
            onClick={() => onOpenChange(false)}
          >
            {t("meetings.retranscribeDialog.cancel")}
          </Button>
          <Button
            className="max-sm:min-h-[44px]"
            onClick={start}
            disabled={scope === "range" && !fieldsValid}
            data-testid="retranscribe-start"
          >
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
            {t("meetings.retranscribe.language")}
          </span>
          <div data-testid="retranscribe-language">
            <Select
              ariaLabel={t("meetings.retranscribe.language")}
              value={language}
              options={languageChoices}
              isClearable={false}
              menuPortal
              onChange={(code) => setLanguage(code ?? AUTO)}
            />
          </div>
        </div>
        <div className="space-y-1">
          <span className="text-xs font-medium text-text/60">
            {t("meetings.retranscribeDialog.model")}
          </span>
          <div data-testid="retranscribe-model">
            <Select
              ariaLabel={t("meetings.retranscribeDialog.model")}
              value={modelId}
              options={options}
              isClearable={false}
              menuPortal
              placeholder={t("meetings.retranscribeDialog.model")}
              onChange={(id) => {
                modelTouched.current = true;
                setModelId(id ?? CONFIGURED);
              }}
            />
          </div>
        </div>
        {durationMs !== null && (
          <div className="space-y-2" data-testid="retranscribe-scope">
            <span className="text-xs font-medium text-text/60">
              {t("meetings.retranscribe.rangeLabel")}
            </span>
            <RadioGroup
              value={scope}
              onChange={(_, data) => setScope(data.value as Scope)}
              aria-label={t("meetings.retranscribe.rangeLabel")}
            >
              <Radio
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                value="all"
                label={t("meetings.retranscribe.rangeAll")}
                data-testid="retranscribe-scope-all"
              />
              <Radio
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                value="range"
                label={t("meetings.retranscribe.rangeOnly")}
                data-testid="retranscribe-scope-range"
              />
            </RadioGroup>
            {scope === "range" && (
              <>
                <AudioRangePicker
                  durationMs={durationMs}
                  value={range}
                  onChange={setRange}
                  handleRef={pickerRef}
                  onValidityChange={setFieldsValid}
                  testId="retranscribe-range"
                />
                <p
                  className="text-sm text-text/80"
                  data-testid="retranscribe-range-hint"
                >
                  {t("meetings.retranscribe.rangeHint")}
                </p>
              </>
            )}
          </div>
        )}
        {suggestion && (
          <Alert variant="info">
            <span data-testid="retranscribe-suggestion">
              {suggestion.downloaded
                ? t("meetings.retranscribe.suggestInstalled", {
                    model: suggestion.name,
                  })
                : t("meetings.retranscribe.suggestMissing", {
                    model: suggestion.name,
                  })}
            </span>
          </Alert>
        )}
        <Alert variant="info">
          <span data-testid="retranscribe-hint">
            {t("meetings.retranscribe.runningHint")}
          </span>
        </Alert>
      </div>
    </Dialog>
  );
};
