import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Cpu } from "lucide-react";
import { commands, type LlmConnection, type LlmModelConfig } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { openSettingsTab } from "@/lib/openSettingsTab";

/**
 * Kompaktes Etikett „Anbieter · Modell“ des Sprachmodells, das übersetzt
 * (das aktive Modell der Fußleiste). Klick öffnet die Modellwahl; sie wechselt
 * das aktive Modell wie die Fußleiste (`llmSetActiveModel`).
 */
export const TranslationModelChip: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
  const selectable = useMemo(() => {
    const on = new Map(
      connections.filter((c) => c.enabled !== false).map((c) => [c.id, c]),
    );
    return models
      .filter((m) => m.enabled !== false && on.has(m.connection_id))
      .map((m) => ({ model: m, connection: on.get(m.connection_id)! }));
  }, [connections, models]);
  const active = selectable.find((s) => s.model.id === activeId) ?? null;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const text = active
    ? `${active.connection.label} · ${modelLabel(active.model.remote_id, active.model.label)}`
    : t("tts.translationModel.none");

  const choose = async (id: string) => {
    setOpen(false);
    const result = await commands.llmSetActiveModel(id);
    if (result.status === "ok") await refreshSettings();
  };

  return (
    <div ref={ref} className="relative min-w-0" data-testid="tts-translation-model">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        title={t("tts.translationModel.hint")}
        className="inline-flex min-h-[44px] max-w-full items-center gap-1.5 rounded-md border border-mid-gray/30 px-2 text-xs text-text/80 hover:bg-mid-gray/10 sm:min-h-[28px]"
      >
        <Cpu size={13} aria-hidden className="shrink-0 text-text/60" />
        <span className="shrink-0 text-text/50">
          {t("tts.translationModel.label")}
        </span>
        <span className="min-w-0 truncate font-medium">{text}</span>
        <ChevronDown size={12} aria-hidden className="shrink-0" />
      </button>
      {open && (
        <div
          role="listbox"
          aria-label={t("tts.translationModel.label")}
          className="absolute start-0 top-full z-40 mt-1 max-h-72 w-72 max-w-[85vw] overflow-auto rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
        >
          {selectable.map(({ model, connection }) => (
            <button
              key={model.id}
              type="button"
              role="option"
              aria-selected={model.id === activeId}
              onClick={() => void choose(model.id)}
              className="flex min-h-[44px] w-full items-center gap-2 px-3 text-start text-xs hover:bg-mid-gray/10 sm:min-h-[32px]"
            >
              <Check
                size={12}
                aria-hidden
                className={model.id === activeId ? "" : "invisible"}
              />
              <span className="min-w-0 truncate">
                {connection.label} · {modelLabel(model.remote_id, model.label)}
              </span>
            </button>
          ))}
          <button
            type="button"
            onClick={() => {
              setOpen(false);
              openSettingsTab("models");
            }}
            className="flex min-h-[44px] w-full items-center border-t border-mid-gray/20 px-3 text-start text-xs text-text/70 hover:bg-mid-gray/10 sm:min-h-[32px]"
          >
            {t("tts.translationModel.settings")}
          </button>
        </div>
      )}
    </div>
  );
};
