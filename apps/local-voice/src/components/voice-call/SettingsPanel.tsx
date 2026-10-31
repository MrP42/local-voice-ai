import React, { useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Switch } from "@fluentui/react-components";
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
import {
  SOUVERAENITAET_STUFEN,
  blockedBy,
  describeModels,
  type ModelDescription,
} from "./orchestration";
import { useSettings } from "@/hooks/useSettings";
import { useComplianceStore } from "@/stores/complianceStore";
import type { LlmConnection, LlmModelConfig } from "@/bindings";

/** Auswahlwert für „kein Denker“ (die Auswahl kennt keinen leeren Wert). */
const DENKER_OFF = "off";

const TAB_IDS: VoiceSettingsTab[] = ["talk", "voice", "recognition", "models"];

const Field: React.FC<{
  label: string;
  hint?: string;
  children: React.ReactNode;
}> = ({ label, hint, children }) => (
  <div className="flex flex-col gap-1">
    <span className="text-[13px] font-semibold">{label}</span>
    {children}
    {hint && <span className="text-xs text-text/60">{hint}</span>}
  </div>
);

const Note: React.FC<{ title: string; children: React.ReactNode }> = ({
  title,
  children,
}) => (
  <div className="rounded-lg border border-mid-gray/25 bg-mid-gray/10 px-3 py-2.5">
    <div className="font-semibold text-[13px]">{title}</div>
    <div className="text-xs text-text/65 mt-0.5">{children}</div>
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
            <p className="m-0 text-xs text-text/60">
              {t("voiceCall.settings.note")}
            </p>
          </>
        )}
        {tab === "recognition" && (
          <>
            <Field label={t("voiceCall.settings.recognitionModel")}>
              <div className="text-sm">{stt ?? "–"}</div>
            </Field>
            <Note title={t("voiceCall.settings.autoTitle")}>
              {t("voiceCall.settings.autoSoon")}
            </Note>
          </>
        )}
        {tab === "models" && (
          <>
            <Field
              label={t("voiceCall.settings.answerModel")}
              hint={t("voiceCall.settings.answerModelHint")}
            >
              <div className="text-sm">{ready?.llm || llm.label || "–"}</div>
            </Field>
            <Field
              label={t("voiceCall.settings.denker")}
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
              <div
                role="radiogroup"
                aria-label={t("voiceCall.settings.sovereignty")}
                className="flex flex-wrap gap-1.5"
              >
                {SOUVERAENITAET_STUFEN.map((k) => (
                  <Button
                    key={k}
                    size="sm"
                    variant={
                      orch.souveraenitaet === k ? "primary" : "secondary"
                    }
                    role="radio"
                    aria-checked={orch.souveraenitaet === k}
                    type="button"
                    onClick={() => setOrch({ souveraenitaet: k })}
                  >
                    {t(`voiceCall.settings.sovereigntyLevels.${k}`)}
                  </Button>
                ))}
              </div>
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
