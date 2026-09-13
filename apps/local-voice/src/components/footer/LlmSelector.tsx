import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type LlmConnection,
  type LlmModelConfig,
  type LocalLlmStatus,
} from "@/bindings";
import { useSettings } from "../../hooks/useSettings";

/**
 * Das Sprachmodell in der Fußleiste — neben dem Diktatmodell, mit derselben
 * Ampel: grün = geladen, gelb = lädt, grau = nicht geladen, rot = Fehler.
 * Das Dropdown wechselt zwischen allen freigegebenen Modellen eingeschalteter
 * Verbindungen; Cloud-Modelle sind immer "bereit", ein lokales zeigt, ob es
 * im Speicher liegt, und lässt sich dort auch entladen.
 */
export const LlmSelector: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<LocalLlmStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
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
  const activeIsLocal = active?.connection.kind === "local";

  // Status des lokalen Servers alle drei Sekunden — nur wenn das aktive
  // Modell lokal ist; sonst gibt es nichts zu beobachten.
  useEffect(() => {
    if (!activeIsLocal) {
      setStatus(null);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const next = await commands.llmLocalStatus();
        if (!cancelled) setStatus(next);
      } catch {
        // Ohne Backend (Browser-Test) bleibt der Status unbekannt.
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 3000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeIsLocal, activeId]);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const light = (): string => {
    if (!active) return "bg-red-400";
    if (!activeIsLocal) return "bg-green-400";
    switch (status?.phase) {
      case "ready":
        return "bg-green-400";
      case "starting":
        return "bg-yellow-400 animate-pulse";
      case "error":
        return "bg-red-400";
      default:
        return "bg-mid-gray/60";
    }
  };

  const label = (): string => {
    if (!active) return t("llmSelector.none");
    const name = active.model.label;
    if (!activeIsLocal) return name;
    switch (status?.phase) {
      case "starting":
        return t("llmSelector.loading", { name });
      case "error":
        return t("llmSelector.error", { name });
      default:
        return name;
    }
  };

  const choose = async (id: string) => {
    setOpen(false);
    setError(null);
    const result = await commands.llmSetActiveModel(id);
    if (result.status === "error") {
      setError(String(result.error));
      return;
    }
    await refreshSettings();
  };

  const unload = async () => {
    setOpen(false);
    await commands.llmLocalStop();
    setStatus({
      phase: "stopped",
      model_id: null,
      backend: null,
      port: null,
      message: null,
    });
  };

  const title = error ?? (status?.message ?? undefined);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 hover:text-text/80 transition-colors"
        title={title ?? t("llmSelector.title")}
        aria-label={t("llmSelector.title")}
        aria-expanded={open}
        data-llm-selector
      >
        <div className={`w-2 h-2 rounded-full ${light()}`} />
        <span className="max-w-32 truncate">{label()}</span>
        <svg
          className={`w-3 h-3 transition-transform ${open ? "rotate-180" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          aria-hidden="true"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      {open && (
        <div
          className="absolute bottom-full start-0 mb-2 w-72 max-h-[60vh] overflow-y-auto bg-background border border-mid-gray/20 rounded-lg shadow-lg py-2 z-50"
          role="menu"
        >
          {selectable.length === 0 ? (
            <div className="px-3 py-2 text-sm text-text/60">
              {t("llmSelector.empty")}
            </div>
          ) : (
            selectable.map(({ model, connection }) => (
              <div
                key={model.id}
                role="menuitem"
                tabIndex={0}
                onClick={() => void choose(model.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    void choose(model.id);
                  }
                }}
                className={`w-full px-3 py-2 text-start hover:bg-mid-gray/10 transition-colors cursor-pointer focus:outline-none ${
                  model.id === activeId ? "bg-logo-primary/10 text-logo-primary" : ""
                }`}
              >
                <div className="text-sm text-text/80">{model.label}</div>
                <div className="text-xs text-text/50">{connection.label}</div>
              </div>
            ))
          )}
          {activeIsLocal && status?.phase === "ready" && (
            <button
              type="button"
              onClick={() => void unload()}
              className="w-full px-3 py-2 mt-1 border-t border-mid-gray/20 text-start text-sm text-text/70 hover:bg-mid-gray/10"
            >
              {t("llmSelector.unload")}
            </button>
          )}
        </div>
      )}
    </div>
  );
};

export default LlmSelector;
