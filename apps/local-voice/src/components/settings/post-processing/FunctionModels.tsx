import React, { useEffect, useState } from "react";
import { Select as FluentSelect } from "@fluentui/react-components";
import { useTranslation } from "react-i18next";
import { commands, type FunctionModel, type LlmModelConfig } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";

/** Funktionen mit eigener Modellwahl (wie `LLM_FUNCTIONS` in Rust). */
export const LLM_FUNCTIONS = [
  "voice_speaker",
  "meeting_protocol",
  "dictation_refine",
] as const;

type RunFn = (
  call: () => Promise<{ status: string; error?: unknown }>,
) => Promise<boolean>;

interface Props {
  selectable: LlmModelConfig[];
  labelFor: (m: LlmModelConfig) => string;
  run: RunFn;
}

/**
 * „Modelle je Funktion“: jede Aufgabe (Sprachgespräch, Protokoll, Diktat) kann ein
 * eigenes Modell samt Effort haben. Leer = Standardmodell. Gespeichert in den
 * Einstellungen (`function_models`), damit auch Rust (Protokoll, Diktat) sie nutzt.
 */
export const FunctionModels: React.FC<Props> = ({
  selectable,
  labelFor,
  run,
}) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const entries = (getSetting("function_models") ?? {}) as Partial<
    Record<string, FunctionModel>
  >;
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
  const active = selectable.find((m) => m.id === activeId);

  return (
    <SettingsGroup title={t("settings.llm.functions.title")}>
      <p className="px-4 pt-3 text-xs text-text/60">
        {t("settings.llm.functions.description")}
      </p>
      {LLM_FUNCTIONS.map((fn) => (
        <FunctionRow
          key={fn}
          fn={fn}
          entry={entries[fn]}
          selectable={selectable}
          defaultLabel={
            active
              ? t("settings.llm.functions.default", {
                  model: modelLabel(active.remote_id, active.label),
                })
              : t("settings.llm.functions.defaultNone")
          }
          labelFor={labelFor}
          run={run}
        />
      ))}
      <ul
        className="px-4 py-2 text-xs text-text/60 space-y-0.5 list-disc list-inside"
        data-function-notes
      >
        <li>{t("settings.llm.functions.fallback")}</li>
        <li>{t("settings.llm.functions.compliance")}</li>
      </ul>
    </SettingsGroup>
  );
};

const FunctionRow: React.FC<{
  fn: string;
  entry: FunctionModel | undefined;
  selectable: LlmModelConfig[];
  defaultLabel: string;
  labelFor: (m: LlmModelConfig) => string;
  run: RunFn;
}> = ({ fn, entry, selectable, defaultLabel, labelFor, run }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  // Ein Eintrag mit nicht mehr wählbarem Modell gilt als Standard (wie in Rust).
  const model = selectable.find((m) => m.id === entry?.model_id);
  const name = t(`settings.llm.functions.names.${fn}`);
  const baseEffort =
    model?.effort ?? ((getSetting("llm_default_effort") ?? "medium") as string);
  const [efforts, setEfforts] = useState<string[]>([]);

  useEffect(() => {
    let cancelled = false;
    setEfforts([]);
    if (!model) return;
    commands
      .llmModelEfforts(model.connection_id, model.remote_id)
      .then((list) => {
        if (!cancelled) setEfforts(list ?? []);
      })
      .catch(() => {
        // Ohne Backend: keine Auswahl.
      });
    return () => {
      cancelled = true;
    };
  }, [model?.connection_id, model?.remote_id]);

  const save = (modelId: string | null, effort: string | null) =>
    void run(() => commands.llmSetFunctionModel(fn, modelId, effort));

  return (
    <SettingContainer
      title={name}
      description={t(`settings.llm.functions.hints.${fn}`)}
      descriptionMode="tooltip"
      layout="horizontal"
      grouped={true}
    >
      <div className="flex items-center gap-2" data-function-model={fn}>
        <FluentSelect
          value={model?.id ?? ""}
          onChange={(e) => save(e.target.value || null, null)}
          className="w-72"
          aria-label={name}
          data-function-select={fn}
        >
          <option value="">{defaultLabel}</option>
          {selectable.map((m) => (
            <option key={m.id} value={m.id}>
              {labelFor(m)}
            </option>
          ))}
        </FluentSelect>
        {model && efforts.length > 0 && (
          <FluentSelect
            value={entry?.effort ?? ""}
            onChange={(e) => save(model.id, e.target.value || null)}
            className="w-36"
            aria-label={t("settings.llm.functions.effort", { name })}
            data-function-effort={fn}
          >
            <option value="">
              {t("settings.llm.model.effortDefault", {
                level: t(`settings.llm.effortLevels.${baseEffort}`, {
                  defaultValue: baseEffort,
                }),
              })}
            </option>
            {efforts.map((level) => (
              <option key={level} value={level}>
                {t(`settings.llm.effortLevels.${level}`, {
                  defaultValue: level,
                })}
              </option>
            ))}
          </FluentSelect>
        )}
      </div>
    </SettingContainer>
  );
};

export default FunctionModels;
