import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Cpu } from "lucide-react";
import { commands, type LlmConnection, type LlmModelConfig } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { openSettingsTab } from "@/lib/openSettingsTab";

/** Kurzname fürs Etikett: „Claude Opus 5.5“ → „Opus 5.5“. */
export const shortModelName = (name: string): string =>
  name.replace(/^Claude\s+/, "");

/** Höhe, die die Liste unter dem Knopf mindestens braucht (max-h-72 + Rand). */
const LIST_HEIGHT = 300;

/**
 * Kompaktes Etikett mit dem Kurznamen des Modells des Sprachmodells, das übersetzt
 * (das aktive Modell der Fußleiste). Klick öffnet die Modellwahl; sie wechselt
 * das aktive Modell wie die Fußleiste (`llmSetActiveModel`).
 */
export const TranslationModelChip: React.FC<{
  /** Liste am Ende des Knopfs ausrichten (Knopf rechts in der Leiste). */
  alignEnd?: boolean;
}> = ({ alignEnd = false }) => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const [open, setOpen] = useState(false);
  // Die Leiste sitzt unten im Fenster: die Liste öffnet nach oben, außer
  // unten ist genug Platz und oben weniger (Patrick 02.11.).
  const [up, setUp] = useState(true);
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

  const name = active
    ? modelLabel(active.model.remote_id, active.model.label)
    : null;
  const full = active
    ? `${active.connection.label} · ${name}`
    : t("tts.translationModel.none");
  const short = name ? shortModelName(name) : full;

  const toggle = () => {
    if (!open) {
      const rect = ref.current?.getBoundingClientRect();
      if (rect) {
        const below = window.innerHeight - rect.bottom;
        setUp(below < LIST_HEIGHT && rect.top > below);
      }
    }
    setOpen((v) => !v);
  };

  const choose = async (id: string) => {
    setOpen(false);
    const result = await commands.llmSetActiveModel(id);
    if (result.status === "ok") await refreshSettings();
  };

  return (
    <div
      ref={ref}
      className="relative min-w-0 shrink-0"
      data-testid="tts-translation-model"
    >
      <button
        type="button"
        onClick={toggle}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={`${t("tts.translationModel.label")} ${full}`}
        title={`${t("tts.translationModel.label")} ${full}\n${t("tts.translationModel.hint")}`}
        className="flex min-h-[44px] max-w-full items-center gap-1.5 rounded-md border border-mid-gray/50 bg-mid-gray/10 px-2 py-1 text-start text-sm text-text hover:border-logo-primary hover:bg-mid-gray/20 sm:min-h-[32px]"
      >
        <Cpu size={14} aria-hidden className="shrink-0" />
        <span
          className="min-w-0 max-w-[12rem] truncate font-semibold"
          data-testid="tts-translation-model-name"
        >
          {short}
        </span>
        <ChevronDown size={16} aria-hidden className="shrink-0 text-text" />
      </button>
      {open && (
        <div
          role="listbox"
          aria-label={t("tts.translationModel.label")}
          data-placement={up ? "above" : "below"}
          className={`absolute ${alignEnd ? "end-0" : "start-0"} z-40 max-h-72 w-72 max-w-[85vw] overflow-auto ${up ? "bottom-full mb-1" : "top-full mt-1"} rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg`}
        >
          {selectable.map(({ model, connection }) => (
            <button
              key={model.id}
              type="button"
              role="option"
              aria-selected={model.id === activeId}
              onClick={() => void choose(model.id)}
              className="flex min-h-[44px] w-full items-center gap-2 px-3 text-start text-xs hover:bg-mid-gray/10"
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
            className="flex min-h-[44px] w-full items-center border-t border-mid-gray/20 px-3 text-start text-xs text-text/70 hover:bg-mid-gray/10"
          >
            {t("tts.translationModel.settings")}
          </button>
        </div>
      )}
    </div>
  );
};
