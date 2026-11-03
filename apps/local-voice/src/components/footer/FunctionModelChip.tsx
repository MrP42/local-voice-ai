import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Sparkle } from "lucide-react";
import {
  commands,
  type FunctionModel,
  type LlmConnection,
  type LlmModelConfig,
} from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { openSettingsTab } from "@/lib/openSettingsTab";
import { notifyComplianceChanged } from "@/stores/complianceStore";

interface Props {
  /** Funktion in `function_models` (wie `LLM_FUNCTIONS` in Rust). */
  fn: string;
}

/**
 * Sprachmodell EINER Funktion in der Fußleiste (Patrick 02.11.): z. B. beim
 * Vorlesen das Modell für Übersetzen und KI-Dokumente. Die Wahl gilt nur für
 * diese Funktion (`function_models`), nicht global; „Standard“ folgt dem
 * aktiven Modell. Die Liste öffnet nach oben, links am Knopf, und verdeckt
 * damit weder Restzeit noch Seitenfeld der Spielerleiste.
 */
export const FunctionModelChip: React.FC<Props> = ({ fn }) => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const knopfRef = useRef<HTMLButtonElement>(null);
  const menueRef = useRef<HTMLDivElement>(null);
  /** Fest am Fenster verankert: die schmale Leiste rollt seitlich
      (overflow-x) und würde ein absolut gesetztes Menü abschneiden. */
  const [anker, setAnker] = useState<{ left: number; bottom: number } | null>(
    null,
  );

  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const activeId = (getSetting("llm_active_model_id") ?? null) as string | null;
  const entries = (getSetting("function_models") ?? {}) as Partial<
    Record<string, FunctionModel>
  >;

  const selectable = useMemo(() => {
    const on = new Map(
      connections.filter((c) => c.enabled !== false).map((c) => [c.id, c]),
    );
    return models
      .filter((m) => m.enabled !== false && on.has(m.connection_id))
      .map((m) => ({ model: m, connection: on.get(m.connection_id)! }));
  }, [connections, models]);

  // Ein Eintrag mit nicht mehr wählbarem Modell gilt als Standard (wie Rust).
  const own = selectable.find((s) => s.model.id === entries[fn]?.model_id);
  const fallback = selectable.find((s) => s.model.id === activeId) ?? null;
  const shown = own ?? fallback;
  const name = t(`settings.llm.functions.names.${fn}`);
  const short = shown
    ? modelLabel(shown.model.remote_id, shown.model.label)
    : t("footer.functionModel.none");
  const full = shown
    ? `${shown.connection.label} · ${modelLabel(shown.model.remote_id, shown.model.label)}`
    : t("footer.functionModel.none");

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        knopfRef.current?.focus();
        return;
      }
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      const items = Array.from(
        menueRef.current?.querySelectorAll<HTMLButtonElement>("button") ?? [],
      );
      if (items.length === 0) return;
      e.preventDefault();
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      const next =
        e.key === "ArrowDown"
          ? (at + 1) % items.length
          : (at - 1 + items.length) % items.length;
      items[next]?.focus();
    };
    const r = knopfRef.current?.getBoundingClientRect();
    if (r)
      setAnker({
        left: Math.max(8, Math.min(r.left, window.innerWidth - 296)),
        bottom: window.innerHeight - r.top + 8,
      });
    menueRef.current
      ?.querySelector<HTMLButtonElement>('[aria-checked="true"]')
      ?.focus();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const choose = async (modelId: string | null) => {
    setOpen(false);
    setError(null);
    const result = await commands.llmSetFunctionModel(fn, modelId, null);
    if (result.status === "error") {
      setError(String(result.error));
      return;
    }
    await refreshSettings();
    notifyComplianceChanged();
  };

  const title = t("footer.functionModel.title", { name, model: full });

  return (
    <div className="relative" ref={ref} data-function-chip={fn}>
      <button
        ref={knopfRef}
        type="button"
        className="lv-status-btn"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={title}
        title={error ?? title}
        data-testid={`fuss-modell-${fn}`}
      >
        <span className="relative inline-flex">
          <Sparkle size={14} aria-hidden="true" />
          {own && (
            <span
              aria-hidden="true"
              className="absolute -bottom-px -end-px w-1.5 h-1.5 rounded-full ring-1 ring-background bg-logo-primary"
            />
          )}
        </span>
        <span className="lv-status-label max-w-44 truncate">
          <span className="text-text/60">{name}:</span> {short}
        </span>
      </button>
      {open && (
        <div
          ref={menueRef}
          role="menu"
          aria-label={name}
          style={
            anker
              ? { position: "fixed", left: anker.left, bottom: anker.bottom }
              : undefined
          }
          className="lv-fuss-menue absolute bottom-full start-0 mb-2 w-72 max-w-[calc(100vw-16px)] max-h-[60vh] overflow-y-auto bg-background border border-mid-gray/20 rounded-lg shadow-lg py-1 z-50"
          data-testid={`fuss-modell-menue-${fn}`}
        >
          <p className="px-3 pt-1 pb-2 text-xs text-text/60">
            {t(`settings.llm.functions.hints.${fn}`)}
          </p>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={!own}
            className="flex w-full min-h-[44px] items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/10"
            onClick={() => void choose(null)}
            data-function-option=""
          >
            <span className="w-4 shrink-0">
              {!own && <Check size={14} aria-hidden="true" />}
            </span>
            <span className="min-w-0 break-words">
              {fallback
                ? t("settings.llm.functions.default", {
                    model: modelLabel(
                      fallback.model.remote_id,
                      fallback.model.label,
                    ),
                  })
                : t("settings.llm.functions.defaultNone")}
            </span>
          </button>
          {selectable.map(({ model, connection }) => {
            const checked = own?.model.id === model.id;
            return (
              <button
                key={model.id}
                type="button"
                role="menuitemradio"
                aria-checked={checked}
                className="flex w-full min-h-[44px] items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/10"
                onClick={() => void choose(model.id)}
                data-function-option={model.id}
              >
                <span className="w-4 shrink-0">
                  {checked && <Check size={14} aria-hidden="true" />}
                </span>
                <span className="min-w-0">
                  <span className="block break-words">
                    {modelLabel(model.remote_id, model.label)}
                  </span>
                  <span className="block text-xs text-text/50">
                    {connection.label}
                  </span>
                </span>
              </button>
            );
          })}
          <button
            type="button"
            role="menuitem"
            className="w-full min-h-[44px] px-3 py-2 mt-1 border-t border-mid-gray/20 text-start text-sm text-text/70 hover:bg-mid-gray/10"
            onClick={() => {
              setOpen(false);
              openSettingsTab("models", "connections");
            }}
          >
            {t("footer.functionModel.manage")}
          </button>
        </div>
      )}
    </div>
  );
};

export default FunctionModelChip;
