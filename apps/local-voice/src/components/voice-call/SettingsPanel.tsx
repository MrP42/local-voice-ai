import React, { useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Radio, RadioGroup, Switch, useId } from "@fluentui/react-components";
import { X } from "lucide-react";
import {
  useVoiceCallStore,
  type VoiceSettingsTab,
} from "@/stores/voiceCallStore";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";
import { TabList } from "../ui/TabList";
import { Textarea } from "../ui/Textarea";
import { VOICE_MANAGE, useVoiceChoice } from "../settings/tts/useVoiceChoice";
import { useServerPhase } from "../settings/meetings/ReadAloudBar";
import { TEMPO_PRESETS, TEMPOS, tempoOf } from "./tempo";
import { useActiveLlm, useSttModelName } from "./useVoiceInfo";
import { RecognitionLearning } from "./RecognitionLearning";
import {
  KNOWLEDGE_AREAS,
  SOUVERAENITAET_STUFEN,
  blockedBy,
  describeModels,
  speakerModelId,
  type KnowledgeArea,
  type ModelDescription,
} from "./orchestration";
import { useKnowledgeEditor } from "./useKnowledge";
import { useSettings } from "@/hooks/useSettings";
import { useComplianceStore } from "@/stores/complianceStore";
import type { LlmConnection, LlmModelConfig } from "@/bindings";

/** Auswahlwert für „kein Denker“ (die Auswahl kennt keinen leeren Wert). */
const DENKER_OFF = "off";

const TAB_IDS: VoiceSettingsTab[] = [
  "talk",
  "voice",
  "recognition",
  "knowledge",
  "models",
];

const Field: React.FC<{
  label: string;
  hint?: string;
  /** Kennung für den Hinweis (damit ein Feld ihn mit `aria-describedby` binden kann). */
  hintId?: string;
  children: React.ReactNode;
}> = ({ label, hint, hintId, children }) => (
  <div className="flex flex-col gap-1">
    <span className="text-[13px] font-semibold">{label}</span>
    {children}
    {hint && (
      <span id={hintId} className="text-xs text-text-muted">
        {hint}
      </span>
    )}
  </div>
);

export const VoiceTempoPicker: React.FC = () => {
  const { t } = useTranslation();
  const turn = useVoiceCallStore((s) => s.config.turn);
  const setConfig = useVoiceCallStore((s) => s.setConfig);
  const current = tempoOf(turn);
  return (
    <div
      role="radiogroup"
      aria-label={t("voiceCall.settings.tempo")}
      className="flex flex-wrap gap-1.5"
    >
      {TEMPOS.map((k) => (
        <Button
          key={k}
          size="sm"
          variant={current === k ? "primary" : "secondary"}
          role="radio"
          aria-checked={current === k}
          type="button"
          onClick={() =>
            setConfig((c) => ({
              ...c,
              turn: { ...c.turn, ...TEMPO_PRESETS[k] },
            }))
          }
        >
          {t(`voiceCall.tempo.${k}`)}
        </Button>
      ))}
      <Button
        size="sm"
        variant={current === "custom" ? "primary" : "secondary"}
        role="radio"
        aria-checked={current === "custom"}
        disabled={current !== "custom"}
        type="button"
      >
        {t("voiceCall.tempo.custom")}
      </Button>
    </div>
  );
};

/** Auswahl der Stimme: dieselbe Liste wie unter „Vorlesen“. */
export const VoicePicker: React.FC<{ className?: string }> = ({
  className,
}) => {
  const { t } = useTranslation();
  const phase = useServerPhase();
  const { options, voiceValue, applyVoiceValue, openVoiceManagement } =
    useVoiceChoice(phase);
  return (
    <Select
      className={className}
      value={voiceValue}
      options={options}
      isClearable={false}
      menuPortal
      ariaLabel={t("voiceCall.settings.voice")}
      onChange={(value) => {
        if (!value) return;
        if (value === VOICE_MANAGE) openVoiceManagement();
        else applyVoiceValue(value);
      }}
    />
  );
};

/** Die freigegebenen Modelle mit Herkunft (lokal/EU/andere), für die Wahl des Denkers. */
function useVoiceModels(): ModelDescription[] {
  const { getSetting } = useSettings();
  const byModel = useComplianceStore((s) => s.byModel);
  const refresh = useComplianceStore((s) => s.refresh);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  return useMemo(
    () =>
      describeModels(
        { llm_connections: connections, llm_models: models },
        byModel,
      ),
    [connections, models, byModel],
  );
}

/** Eine Zeile des Reiters „Wissen“: Schalter mit einer Zeile Beschreibung darunter. */
const KnowledgeRow: React.FC<{
  id: string;
  label: string;
  hint: string;
  checked: boolean;
  disabled?: boolean;
  nested?: boolean;
  onChange: (checked: boolean) => void;
}> = ({ id, label, hint, checked, disabled, nested, onChange }) => (
  <div className={`flex flex-col ${nested ? "ms-6" : ""}`}>
    <Switch
      checked={checked}
      disabled={disabled}
      onChange={(_e, d) => onChange(d.checked)}
      label={label}
      aria-describedby={id}
    />
    <span id={id} className="ps-[52px] -mt-1 text-xs text-text-muted">
      {hint}
    </span>
  </div>
);

/** Reiter „Wissen“: was das Modell im Gespräch nachschlagen darf. */
const KnowledgeTab: React.FC = () => {
  const { t } = useTranslation();
  const { knowledge, save, toggle } = useKnowledgeEditor();
  const useTools = useVoiceCallStore((s) => s.config.orchestration.useTools);
  const ids = useId("knowledge");
  return (
    <>
      <p className="m-0 text-[13px]">{t("voiceCall.knowledge.intro")}</p>
      {!useTools && (
        <p
          role="note"
          className="m-0 rounded-lg border border-mid-gray/30 bg-mid-gray/10 px-3 py-2 text-xs"
        >
          {t("voiceCall.knowledge.toolsOff")}
        </p>
      )}
      {KNOWLEDGE_AREAS.map((area) => (
        <React.Fragment key={area}>
          <KnowledgeRow
            id={`${ids}-${area}`}
            label={t(`voiceCall.knowledge.areas.${area}.label`)}
            hint={t(`voiceCall.knowledge.areas.${area}.hint`)}
            checked={knowledge.areas.includes(area)}
            onChange={(on) => toggle(area, on)}
          />
          {area === "meetings" && (
            <KnowledgeRow
              id={`${ids}-transcripts`}
              nested
              label={t("voiceCall.knowledge.areas.transcripts.label")}
              hint={t("voiceCall.knowledge.areas.transcripts.hint")}
              checked={
                knowledge.transcripts && knowledge.areas.includes("meetings")
              }
              disabled={!knowledge.areas.includes("meetings")}
              onChange={(on) => save("voice_call_read_transcripts", on)}
            />
          )}
        </React.Fragment>
      ))}
    </>
  );
};

export const SettingsPanel: React.FC = () => {
  const { t } = useTranslation();
  const config = useVoiceCallStore((s) => s.config);
  const tab = useVoiceCallStore((s) => s.settingsTab);
  const setTab = useVoiceCallStore((s) => s.setSettingsTab);
  const close = useVoiceCallStore((s) => s.closeSettings);
  const setConfig = useVoiceCallStore((s) => s.setConfig);
  const resetConfig = useVoiceCallStore((s) => s.resetConfig);
  const llm = useActiveLlm();
  const stt = useSttModelName();
  const ready = useVoiceCallStore((s) => s.ready);
  const models = useVoiceModels();
  const { getSetting } = useSettings();
  const activeId = speakerModelId({
    function_models: getSetting("function_models"),
    llm_active_model_id: getSetting("llm_active_model_id"),
    llm_models: getSetting("llm_models"),
    llm_connections: getSetting("llm_connections"),
  });
  const speaker = models.find((m) => m.id === activeId);
  const speakerBlocked =
    !!speaker && blockedBy(speaker, config.orchestration.souveraenitaet);
  const denkerHintId = useId("denker-hint");
  const orch = config.orchestration;
  const setOrch = (patch: Partial<typeof orch>) =>
    setConfig((c) => ({
      ...c,
      orchestration: { ...c.orchestration, ...patch },
    }));
  const denker = models.find((m) => m.id === orch.denkerModelId);
  const denkerBlocked = !!denker && blockedBy(denker, orch.souveraenitaet);
  const setTurn = (patch: Partial<typeof config.turn>) =>
    setConfig((c) => ({ ...c, turn: { ...c.turn, ...patch } }));

  return (
    <aside
      className="flex flex-col min-h-0 w-[380px] shrink-0 border-s border-mid-gray/20 bg-background max-[820px]:absolute max-[820px]:inset-0 max-[820px]:w-auto max-[820px]:border-s-0 z-20"
      aria-label={t("voiceCall.settings.title")}
    >
      <div className="flex items-center h-11 ps-4 pe-2 border-b border-mid-gray/20">
        <h2 className="m-0 text-sm font-semibold">
          {t("voiceCall.settings.title")}
        </h2>
        <button
          type="button"
          className="ms-auto grid place-items-center w-7 h-7 rounded cursor-pointer hover:bg-mid-gray/15"
          aria-label={t("voiceCall.settings.close")}
          onClick={close}
        >
          <X className="w-4 h-4" aria-hidden="true" />
        </button>
      </div>
      <TabList
        tabs={TAB_IDS.map((id) => ({
          id,
          label: t(`voiceCall.settings.tabs.${id}`),
        }))}
        value={tab}
        onChange={setTab}
        compact
        ariaLabel={t("voiceCall.settings.title")}
        className="px-2 border-b border-mid-gray/20"
      />
      <div className="flex-1 overflow-y-auto px-4 py-3 flex flex-col gap-3.5">
        {tab === "talk" && (
          <>
            <Field label={t("voiceCall.settings.prompt")}>
              <Textarea
                rows={6}
                value={config.systemPrompt}
                onChange={(e) =>
                  setConfig((c) => ({ ...c, systemPrompt: e.target.value }))
                }
              />
            </Field>
            <Field
              label={t("voiceCall.settings.tempo")}
              hint={t("voiceCall.settings.tempoHint", {
                end: config.turn.endMs,
                early: config.turn.earlyEndMs,
              })}
            >
              <VoiceTempoPicker />
            </Field>
            <details>
              <summary className="cursor-pointer font-semibold text-[13px]">
                {t("voiceCall.settings.fine")}
              </summary>
              <div className="mt-2 flex flex-col gap-3">
                <label className="flex flex-col gap-1 text-[13px]">
                  {t("voiceCall.settings.endMs", { ms: config.turn.endMs })}
                  <input
                    type="range"
                    min={400}
                    max={1500}
                    step={50}
                    value={config.turn.endMs}
                    onChange={(e) =>
                      setTurn({
                        endMs: Number(e.target.value),
                        earlyEndMs: Math.min(
                          config.turn.earlyEndMs,
                          Number(e.target.value) - 100,
                        ),
                      })
                    }
                  />
                </label>
                <label className="flex flex-col gap-1 text-[13px]">
                  {t("voiceCall.settings.earlyMs", {
                    ms: config.turn.earlyEndMs,
                  })}
                  <input
                    type="range"
                    min={200}
                    max={Math.max(300, config.turn.endMs - 100)}
                    step={20}
                    value={config.turn.earlyEndMs}
                    onChange={(e) =>
                      setTurn({ earlyEndMs: Number(e.target.value) })
                    }
                  />
                </label>
              </div>
            </details>
            <Switch
              checked={config.turn.bargeIn}
              onChange={(_e, d) => setTurn({ bargeIn: d.checked })}
              label={t("voiceCall.settings.bargeIn")}
            />
            <div>
              <Button size="sm" type="button" onClick={resetConfig}>
                {t("voiceCall.settings.reset")}
              </Button>
            </div>
          </>
        )}
        {tab === "voice" && (
          <>
            <Field
              label={t("voiceCall.settings.voice")}
              hint={t("voiceCall.settings.voiceHint")}
            >
              <VoicePicker />
            </Field>
            <p className="m-0 text-xs text-text-muted">
              {t("voiceCall.settings.note")}
            </p>
          </>
        )}
        {tab === "recognition" && (
          <>
            <Field label={t("voiceCall.settings.recognitionModel")}>
              <div className="text-sm">{stt ?? "–"}</div>
            </Field>
            <RecognitionLearning />
          </>
        )}
        {tab === "knowledge" && <KnowledgeTab />}
        {tab === "models" && (
          <>
            <Field
              label={t("voiceCall.settings.answerModel")}
              hint={t("voiceCall.settings.answerModelHint")}
            >
              <div className="text-sm">{ready?.llm || llm.label || "–"}</div>
            </Field>
            {speakerBlocked && (
              <div
                role="alert"
                className="rounded-lg border border-mid-gray/30 bg-mid-gray/10 px-3 py-2 text-xs"
              >
                {t("voiceCall.settings.speakerBlocked")}
              </div>
            )}
            <Field
              label={t("voiceCall.settings.denker")}
              hintId={denkerHintId}
              hint={
                denkerBlocked
                  ? t("voiceCall.settings.denkerBlocked")
                  : t("voiceCall.settings.denkerHint")
              }
            >
              <Select
                value={orch.denkerModelId || DENKER_OFF}
                options={[
                  {
                    value: DENKER_OFF,
                    label: t("voiceCall.settings.denkerOff"),
                  },
                  ...models.map((m) => ({ value: m.id, label: m.label })),
                ]}
                isClearable={false}
                menuPortal
                ariaLabel={t("voiceCall.settings.denker")}
                ariaDescribedBy={denkerHintId}
                onChange={(value) =>
                  setOrch({
                    denkerModelId: !value || value === DENKER_OFF ? "" : value,
                  })
                }
              />
            </Field>
            <Field
              label={t("voiceCall.settings.sovereignty")}
              hint={t(
                `voiceCall.settings.sovereigntyHint.${orch.souveraenitaet}`,
              )}
            >
              <RadioGroup
                layout="horizontal"
                aria-label={t("voiceCall.settings.sovereignty")}
                value={orch.souveraenitaet}
                onChange={(_e, d) => {
                  const next = SOUVERAENITAET_STUFEN.find((k) => k === d.value);
                  if (next) setOrch({ souveraenitaet: next });
                }}
              >
                {SOUVERAENITAET_STUFEN.map((k) => (
                  <Radio
                    key={k}
                    value={k}
                    label={t(`voiceCall.settings.sovereigntyLevels.${k}`)}
                  />
                ))}
              </RadioGroup>
            </Field>
            <Field
              label={t("voiceCall.settings.tools")}
              hint={t("voiceCall.settings.toolsHint")}
            >
              <Switch
                checked={orch.useTools}
                onChange={(_e, d) => setOrch({ useTools: d.checked })}
                aria-label={t("voiceCall.settings.tools")}
              />
            </Field>
          </>
        )}
      </div>
    </aside>
  );
};
