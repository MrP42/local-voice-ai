import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type LlmConnection,
  type LlmModelConfig,
  type BudgetState,
  type LocalLlmStatus,
} from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { ModelBadges, reasonsText } from "../compliance/ModelBadges";
import { displayModelName } from "@/lib/modelNames";
import {
  COMPLIANCE_CHANGED,
  notifyComplianceChanged,
  useComplianceStore,
} from "@/stores/complianceStore";

/**
 * Das Sprachmodell in der Fußleiste — neben dem Diktatmodell, mit derselben
 * Ampel: grün = geladen, gelb = lädt, grau = nicht geladen, rot = Fehler.
 * Das Dropdown wechselt zwischen allen freigegebenen Modellen eingeschalteter
 * Verbindungen; Cloud-Modelle sind immer "bereit", ein lokales zeigt, ob es
 * im Speicher liegt, und lässt sich dort auch entladen.
 *
 * Seit 0.21.7 auch das Ollama-Modell der Nachbearbeitung (vorher ein eigenes
 * Symbol oben auf der Vorlesen-Seite): Ampel gelb pulsierend, solange es
 * arbeitet, im Menü „vorwärmen“ und „entladen“.
 */
export const LlmSelector: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<LocalLlmStatus | null>(null);
  const [budget, setBudget] = useState<BudgetState | null>(null);
  /** Verbrauch des aktiven Modells: dieser Monat und gesamt. */
  const [spend, setSpend] = useState<{
    month: { tokensIn: number; tokensOut: number; cost: number; calls: number };
    all: { tokensIn: number; tokensOut: number; cost: number; calls: number };
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Ollama: was im Speicher liegt, ob gerade gearbeitet wird, letzter Fehler. */
  const [ollamaLoaded, setOllamaLoaded] = useState<string[]>([]);
  const [ollamaBusy, setOllamaBusy] = useState(false);
  const [ollamaError, setOllamaError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  // Regelwerk: Wolke/Flagge hinter dem Namen, gesperrte Modelle ausgegraut.
  const { byModel, refresh: refreshCompliance } = useComplianceStore();
  useEffect(() => {
    void refreshCompliance();
    const onChange = () => void refreshCompliance();
    window.addEventListener(COMPLIANCE_CHANGED, onChange);
    return () => window.removeEventListener(COMPLIANCE_CHANGED, onChange);
  }, [refreshCompliance]);

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
  const defaultEffort = (getSetting("llm_default_effort") ?? "medium") as string;
  const isCli = (c: LlmConnection) =>
    c.kind === "claude_cli" || c.kind === "codex_cli";
  /** Effort, mit dem das Modell laeuft (eigener oder Standard; nur Abo/CLI). */
  const effortOf = (m: LlmModelConfig, c: LlmConnection): string | null =>
    isCli(c) ? m.effort || defaultEffort : null;
  const effortText = (e: string | null) =>
    e ? t(`settings.llm.effortLevels.${e}`, { defaultValue: e }) : null;
  const activeIsLocal = active?.connection.kind === "local";
  const activeConnectionId = active?.connection.id ?? null;

  // Budgetstand der aktiven Verbindung — alle 30 Sekunden, das reicht: ein
  // Aufruf kostet Cent, kein Budget kippt binnen Sekunden.
  useEffect(() => {
    if (!activeConnectionId) {
      setBudget(null);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const result = await commands.usageBudgetStates();
        if (cancelled || result.status !== "ok") return;
        setBudget(
          result.data.find((b) => b.connection_id === activeConnectionId) ??
            null,
        );
      } catch {
        // Ohne Backend bleibt das Budget unbekannt.
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 30_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeConnectionId]);

  // Verbrauch des aktiven Modells (Monat, gesamt) fuer Anzeige und Tooltip.
  useEffect(() => {
    if (!activeId) {
      setSpend(null);
      return;
    }
    let cancelled = false;
    const pick = (s: { by_model: { key: string; prompt_tokens: number; completion_tokens: number; cost_micro: number; calls: number }[] }) => {
      const b = s.by_model.find((x) => x.key === activeId);
      return {
        tokensIn: b?.prompt_tokens ?? 0,
        tokensOut: b?.completion_tokens ?? 0,
        cost: b?.cost_micro ?? 0,
        calls: b?.calls ?? 0,
      };
    };
    const tick = async () => {
      try {
        const [m, a] = await Promise.all([
          commands.usageSummary("month"),
          commands.usageSummary("all"),
        ]);
        if (cancelled || m.status !== "ok" || a.status !== "ok") return;
        setSpend({ month: pick(m.data), all: pick(a.data) });
      } catch {
        // Ohne Backend kein Verbrauch.
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 30_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeId]);

  const usd = (micro: number) =>
    `${(micro / 1_000_000).toLocaleString("de-DE", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })} $`;
  const tok = (n: number) => n.toLocaleString("de-DE");
  const spendTitle = spend
    ? t("llmSelector.spendTitle", {
        monthIn: tok(spend.month.tokensIn),
        monthOut: tok(spend.month.tokensOut),
        monthCost: usd(spend.month.cost),
        monthCalls: spend.month.calls,
        allIn: tok(spend.all.tokensIn),
        allOut: tok(spend.all.tokensOut),
        allCost: usd(spend.all.cost),
        allCalls: spend.all.calls,
      }) +
      (active && isCli(active.connection)
        ? `\n${t("meetings.docUsage.subscriptionHint")}`
        : "")
    : undefined;

  const budgetRatio = budget?.ratio ?? null;
  const budgetWarning = budgetRatio !== null && budgetRatio >= 0.8;
  const budgetExceeded = budgetRatio !== null && budgetRatio >= 1;

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

  // Ollama: Ereignis waehrend der Arbeit, dazu eine Abfrage alle zehn
  // Sekunden (Ollama entlaedt nach eigener Frist, ohne dass die App es erfaehrt).
  // Billig: ohne Ollama als Nachbearbeitung antwortet der Befehl sofort leer.
  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      void commands
        .llmPs()
        .then((loaded) => {
          if (!cancelled) setOllamaLoaded(Array.isArray(loaded) ? loaded : []);
        })
        .catch(() => {
          // Ohne Backend (Browser-Test) bleibt nichts geladen.
        });
    };
    poll();
    const timer = window.setInterval(poll, 10_000);
    const un = listen<{ busy: boolean; error?: string | null }>(
      "llm-activity",
      (e) => {
        setOllamaBusy(e.payload.busy);
        if (!e.payload.busy) {
          setOllamaError(e.payload.error ?? null);
          poll();
        }
      },
    );
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      void un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const light = (): string => {
    // Arbeitet gerade (Uebersetzen, Zusammenfassen): das gilt vor allem anderen.
    if (ollamaBusy) return "bg-yellow-400 animate-pulse";
    if (!active) return "bg-red-400";
    // Budget schlaegt Ladezustand: ein gesperrtes Modell ist nicht "gruen".
    if (budgetExceeded && budget?.enforced) return "bg-red-400";
    if (budgetWarning) return "bg-yellow-400";
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
    const effort = effortText(effortOf(active.model, active.connection));
    const name = `${displayModelName(active.model.remote_id)}${effort ? ` (${effort})` : ""}`;
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
    notifyComplianceChanged();
  };

  /** Ollama-Modell laden, ohne etwas zu erzeugen (vor mehreren Aufträgen). */
  const warmOllama = async () => {
    setOpen(false);
    setOllamaError(null);
    setOllamaBusy(true);
    const result = await commands.llmWarm();
    setOllamaBusy(false);
    if (result.status === "error") setOllamaError(result.error);
    const now = await commands.llmPs();
    setOllamaLoaded(Array.isArray(now) ? now : []);
  };

  /** Alles, was Ollama gerade hält, aus dem Speicher nehmen. */
  const unloadOllama = async () => {
    setOpen(false);
    setOllamaError(null);
    const result = await commands.llmUnload();
    if (result.status === "error") setOllamaError(result.error);
    const now = await commands.llmPs();
    setOllamaLoaded(Array.isArray(now) ? now : []);
  };

  const showOllama =
    active?.connection.kind === "ollama" || ollamaLoaded.length > 0;

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

  const budgetTitle =
    budget && budget.limit_micro !== null
      ? t(
          budgetExceeded ? "llmSelector.budgetExceeded" : "llmSelector.budget",
          {
            spent: (budget.spent_micro / 1_000_000)
              .toFixed(2)
              .replace(".", ","),
            limit: (budget.limit_micro / 1_000_000)
              .toFixed(2)
              .replace(".", ","),
            percent: Math.round(budgetRatio! * 100),
          },
        )
      : undefined;
  const ollamaTitle = ollamaBusy
    ? t("tts.llm.busy")
    : (ollamaError ??
      (ollamaLoaded.length > 0
        ? t("tts.llm.loaded", { models: ollamaLoaded.join(", ") })
        : undefined));
  const title =
    error ??
    ollamaTitle ??
    status?.message ??
    budgetTitle ??
    spendTitle ??
    undefined;

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
        <span className="max-w-56 truncate">{label()}</span>
        {spend && spend.month.cost > 0 && (
          <span className="text-xs tabular-nums text-text/60" data-llm-spend>
            {usd(spend.month.cost)}
          </span>
        )}
        {active && <ModelBadges assessment={byModel[active.model.id]} />}
        {budgetWarning && (
          <span
            className={`text-xs tabular-nums ${budgetExceeded ? "text-red-400" : "text-yellow-500"}`}
            data-llm-budget
          >
            {Math.round(budgetRatio! * 100)}%
          </span>
        )}
        <svg
          className={`w-3 h-3 transition-transform ${open ? "rotate-180" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
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
            selectable.map(({ model, connection }) => {
              const assessment = byModel[model.id];
              // Gesperrt: sichtbar mit Grund, aber nicht waehlbar.
              const blocked = assessment?.verdict === "blocked";
              return (
                <div
                  key={model.id}
                  role="menuitem"
                  tabIndex={blocked ? -1 : 0}
                  aria-disabled={blocked || undefined}
                  data-llm-option={model.id}
                  title={blocked ? reasonsText(t, assessment) : undefined}
                  onClick={() => {
                    if (!blocked) void choose(model.id);
                  }}
                  onKeyDown={(e) => {
                    if (!blocked && (e.key === "Enter" || e.key === " ")) {
                      e.preventDefault();
                      void choose(model.id);
                    }
                  }}
                  className={`w-full px-3 py-2 text-start transition-colors focus:outline-none ${
                    blocked
                      ? "cursor-not-allowed opacity-50"
                      : "cursor-pointer hover:bg-mid-gray/10"
                  } ${
                    model.id === activeId
                      ? "bg-logo-primary/10 text-logo-primary"
                      : ""
                  }`}
                >
                  <div className="flex items-center gap-2 text-sm text-text/80">
                    <span className="min-w-0 truncate">
                      {displayModelName(model.remote_id)}
                    </span>
                    {effortOf(model, connection) && (
                      <span className="text-xs text-text/50" data-llm-effort>
                        ({effortText(effortOf(model, connection))})
                      </span>
                    )}
                    <ModelBadges assessment={assessment} />
                  </div>
                  <div className="text-xs text-text/50">{connection.label}</div>
                </div>
              );
            })
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
          {showOllama && (
            <div
              className="mt-1 border-t border-mid-gray/20 pt-1"
              data-llm-ollama
            >
              <div
                className={`px-3 py-1 text-xs ${ollamaError ? "text-orange-500" : "text-text/60"}`}
              >
                {ollamaError ??
                  (ollamaBusy
                    ? t("tts.llm.busy")
                    : ollamaLoaded.length > 0
                      ? t("llmSelector.ollamaLoaded", {
                          models: ollamaLoaded.join(", "),
                        })
                      : t("llmSelector.ollamaEmpty"))}
              </div>
              <button
                type="button"
                role="menuitem"
                onClick={() => void warmOllama()}
                disabled={ollamaBusy}
                title={t("llmSelector.ollamaWarmHint")}
                className="w-full px-3 py-2 text-start text-sm text-text/70 hover:bg-mid-gray/10 disabled:opacity-50"
                data-llm-warm
              >
                {t("tts.llm.warm")}
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => void unloadOllama()}
                disabled={ollamaBusy || ollamaLoaded.length === 0}
                title={t("llmSelector.ollamaUnloadHint")}
                className="w-full px-3 py-2 text-start text-sm text-text/70 hover:bg-mid-gray/10 disabled:opacity-50"
                data-llm-unload-ollama
              >
                {t("tts.llm.unload")}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default LlmSelector;
