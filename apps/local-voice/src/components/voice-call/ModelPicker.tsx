import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
} from "@fluentui/react-components";
import { ChevronDown, Settings } from "lucide-react";
import {
  commands,
  type LlmConnection,
  type LlmModelConfig,
} from "@/bindings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { useSettings } from "@/hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { openSettingsTab } from "@/lib/openSettingsTab";

const isCli = (c: LlmConnection) =>
  c.kind === "claude_cli" || c.kind === "codex_cli";

/**
 * Modellauswahl im Chatfeld („Claude Opus 5.5 Mittel ⌄“ wie bei ChatGPT). Die Wahl gilt nur für
 * den Sprecher der Gespräche: Modell und Effort stehen in `CallConfig.orchestration`
 * (`speakerModelId`, `speakerEffort`, für alle Gespräche, nicht je Unterhaltung). Leer heißt: das
 * globale aktive Modell der Fußleiste; dieses bleibt unberührt. Die Liste der Modelle und die
 * Effort-Stufen stammen aus denselben Einstellungen und Befehlen wie bei der Fußleiste.
 */
export const ModelPicker: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const speakerModelId = useVoiceCallStore(
    (s) => s.config.orchestration.speakerModelId,
  );
  const speakerEffort = useVoiceCallStore(
    (s) => s.config.orchestration.speakerEffort,
  );
  const setConfig = useVoiceCallStore((s) => s.setConfig);
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
  const defaultEffort = (getSetting("llm_default_effort") ??
    "medium") as string;
  const [efforts, setEfforts] = useState<string[]>([]);

  const selectable = useMemo(() => {
    const on = new Map(
      connections.filter((c) => c.enabled !== false).map((c) => [c.id, c]),
    );
    return models
      .filter((m) => m.enabled !== false && on.has(m.connection_id))
      .map((model) => ({ model, connection: on.get(model.connection_id)! }));
  }, [connections, models]);
  const active =
    selectable.find((s) => s.model.id === speakerModelId) ??
    selectable.find((s) => s.model.id === activeId) ??
    null;
  const activeModelId = active?.model.id;
  const activeConnectionId = active?.model.connection_id;
  const activeRemoteId = active?.model.remote_id;

  useEffect(() => {
    if (!activeConnectionId || !activeRemoteId) {
      setEfforts([]);
      return;
    }
    let cancelled = false;
    commands
      .llmModelEfforts(activeConnectionId, activeRemoteId)
      .then((list) => {
        if (!cancelled) setEfforts(Array.isArray(list) ? list : []);
      })
      .catch(() => {
        if (!cancelled) setEfforts([]);
      });
    return () => {
      cancelled = true;
    };
  }, [activeModelId, activeConnectionId, activeRemoteId]);

  const effortText = (e: string) =>
    t(`settings.llm.effortLevels.${e}`, { defaultValue: e });
  const chosen = active?.model.id === speakerModelId;
  const activeEffort = !active
    ? null
    : !isCli(active.connection)
      ? null
      : chosen && speakerEffort
        ? speakerEffort
        : active.model.effort || defaultEffort;
  const setSpeaker = (modelId: string, effort: string) =>
    setConfig((c) => ({
      ...c,
      orchestration: {
        ...c.orchestration,
        speakerModelId: modelId,
        speakerEffort: effort,
      },
    }));
  const choose = (id: string) => setSpeaker(id, "");
  const chooseEffort = (effort: string) => {
    if (active) setSpeaker(active.model.id, effort);
  };

  const name = active
    ? modelLabel(active.model.remote_id, active.model.label)
    : t("voiceCall.model.none");

  return (
    <Menu
      checkedValues={{
        model: activeModelId ? [activeModelId] : [],
        effort: activeEffort ? [activeEffort] : [],
      }}
      onCheckedValueChange={(_e, data) => {
        const [value] = data.checkedItems;
        if (!value) return;
        if (data.name === "model") choose(value);
        else chooseEffort(value);
      }}
    >
      <MenuTrigger disableButtonEnhancement>
        <button
          type="button"
          data-testid="model-picker"
          aria-label={t("voiceCall.model.aria", {
            model: name,
            effort: activeEffort ? effortText(activeEffort) : "",
          })}
          title={t("voiceCall.model.title")}
          className="inline-flex items-center gap-1 h-8 min-w-8 max-w-full rounded-full px-2 text-xs cursor-pointer hover:bg-mid-gray/15 border border-transparent focus-visible:border-logo-primary"
        >
          <span className="truncate font-semibold text-text/90">{name}</span>
          {activeEffort && (
            <span className="shrink-0 capitalize text-text/55 max-sm:hidden">
              {effortText(activeEffort)}
            </span>
          )}
          <ChevronDown
            className="w-3 h-3 shrink-0 text-text/50"
            aria-hidden="true"
          />
        </button>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuGroup>
            <MenuGroupHeader>{t("voiceCall.model.models")}</MenuGroupHeader>
            {selectable.length === 0 && (
              <MenuItem disabled>{t("voiceCall.model.none")}</MenuItem>
            )}
            {selectable.map(({ model }) => (
              <MenuItemRadio key={model.id} name="model" value={model.id}>
                {modelLabel(model.remote_id, model.label)}
              </MenuItemRadio>
            ))}
          </MenuGroup>
          {efforts.length > 0 && (
            <>
              <MenuDivider />
              <MenuGroup>
                <MenuGroupHeader>
                  {t("voiceCall.model.effort")}
                </MenuGroupHeader>
                {efforts.map((e) => (
                  <MenuItemRadio key={e} name="effort" value={e}>
                    {effortText(e)}
                  </MenuItemRadio>
                ))}
              </MenuGroup>
            </>
          )}
          <MenuDivider />
          <MenuItem
            icon={<Settings size={16} aria-hidden="true" />}
            onClick={() => openSettingsTab("models", "connections")}
          >
            {t("voiceCall.model.allSettings")}
          </MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>
  );
};
