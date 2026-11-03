import React, {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type FunctionModel,
  type LlmConnection,
  type LlmModelConfig,
  type BudgetState,
  type LocalLlmStatus,
} from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { Sparkle } from "lucide-react";
import { RoleTip } from "./RoleTip";
import { ModelBadges, reasonsText } from "../compliance/ModelBadges";
import { modelLabel } from "@/lib/modelNames";
import { openSettingsTab } from "@/lib/openSettingsTab";
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
 *
 * Mit `fn` zeigt derselbe Knopf das Modell EINER Funktion (`function_models`,
 * z. B. `read_aloud` beim Vorlesen, `meeting_protocol` bei Aufnahmen):
 * gleiche Darstellung wie in allen anderen Modulen (Patrick 03.11.), die Wahl
 * gilt aber nur für diese Funktion; „Standard“ folgt dem aktiven Modell.
 */
export const LlmSelector: React.FC<{ fn?: string }> = ({ fn }) => {
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
  const [ollamaBusyRaw, setOllamaBusy] = useState(false);
  const [ollamaError, setOllamaError] = useState<string | null>(null);
  /** Rechtsklick auf das Modellfeld: ein Menue mit dem Sprung zu den Anbietern. */
  const [contextAt, setContextAt] = useState<{ x: number; y: number } | null>(
    null,
  );
  /** Suchtext im Menue: filtert beim Tippen nach Name, Kennung und Verbindung. */
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const knopfRef = useRef<HTMLButtonElement>(null);
  const menueRef = useRef<HTMLDivElement>(null);
  /** Fest am Fenster verankert: die schmale Fußleiste rollt seitlich
      (overflow-x) und würde ein absolut gesetztes Menü abschneiden. */
  const [anker, setAnker] = useState<{ left: number; bottom: number } | null>(
    null,
  );
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
  const globalActiveId = (getSetting("llm_active_model_id") ?? null) as
    string | null;
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

  const globalActive =
    selectable.find((s) => s.model.id === globalActiveId) ?? null;
  // Funktion: eigener Eintrag; ist dessen Modell nicht mehr wählbar, gilt der
  // Standard (wie Rust `for_function`).
  const own = fn
    ? (selectable.find((s) => s.model.id === entries[fn]?.model_id) ?? null)
    : null;
  /** Das Modell, das der Knopf zeigt (und dessen Ampel, Budget, Verbrauch). */
  const active = own ?? globalActive;
  const activeId = active?.model.id ?? null;
  /** In der Liste markiert: bei einer Funktion nur die eigene Wahl. */
  const checkedId = fn ? (own?.model.id ?? null) : globalActiveId;

  const needle = query.trim().toLowerCase();
  const visible = needle
    ? selectable.filter(({ model, connection }) =>
        [
          modelLabel(model.remote_id, model.label),
          model.remote_id,
          connection.label,
        ]
          .join(" ")
          .toLowerCase()
          .includes(needle),
      )
    : selectable;
  const defaultEffort = (getSetting("llm_default_effort") ??
    "medium") as string;
  const isCli = (c: LlmConnection) =>
    c.kind === "claude_cli" || c.kind === "codex_cli";
  /** Effort, mit dem das Modell laeuft (eigener oder Standard; nur Abo/CLI). */
  const effortOf = (m: LlmModelConfig, c: LlmConnection): string | null =>
    isCli(c) ? m.effort || defaultEffort : null;
  const effortText = (e: string | null) =>
    e ? t(`settings.llm.effortLevels.${e}`, { defaultValue: e }) : null;
  const activeIsLocal = active?.connection.kind === "local";
  /** Der lokale Server läuft mit genau einem Modell: dessen Zustand gilt für
      eine Funktion nur, wenn sie dieses Modell zeigt (sonst grau). */
  const fitsShown = (st: LocalLlmStatus): boolean =>
    !own ||
    own === globalActive ||
    (!!st.model_id &&
      [own.model.id, own.model.remote_id].includes(st.model_id));
  /** Ollama-Aktivität betrifft eine Funktion nur, wenn sie Ollama nutzt. */
  const ollamaRelevant = !fn || active?.connection.kind === "ollama";
  const ollamaBusy = ollamaBusyRaw && ollamaRelevant;
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
    const pick = (s: {
      by_model: {
        key: string;
        prompt_tokens: number;
        completion_tokens: number;
        cost_micro: number;
        calls: number;
      }[];
    }) => {
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
        if (!cancelled) setStatus(fitsShown(next) ? next : null);
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
    if (!contextAt) return;
    const close = () => setContextAt(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", close);
    };
  }, [contextAt]);

  const openProviderSettings = () => {
    setOpen(false);
    setContextAt(null);
    openSettingsTab("models", "connections");
  };

  useEffect(() => {
    setQuery("");
  }, [open]);

  // Vor dem ersten Zeichnen messen, sonst springt das Menü.
  useLayoutEffect(() => {
    if (!open) {
      setAnker(null);
      return;
    }
    const r = knopfRef.current?.getBoundingClientRect();
    if (r)
      setAnker({
        left: Math.max(8, Math.min(r.left, window.innerWidth - 296)),
        bottom: window.innerHeight - r.top + 8,
      });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    // Fest verankert: bei Größenänderung oder Rollen der Leiste schließen,
    // statt neben dem Knopf stehen zu bleiben.
    const zu = () => setOpen(false);
    const onScroll = (event: Event) => {
      if (!menueRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (event.defaultPrevented) return;
        setOpen(false);
        knopfRef.current?.focus();
        return;
      }
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      const items = Array.from(
        menueRef.current?.querySelectorAll<HTMLElement>(
          '[role^="menuitem"]:not([aria-disabled="true"]):not(:disabled)',
        ) ?? [],
      );
      if (items.length === 0) return;
      event.preventDefault();
      const at = items.indexOf(document.activeElement as HTMLElement);
      const next =
        event.key === "ArrowDown"
          ? (at + 1) % items.length
          : at <= 0
            ? items.length - 1
            : at - 1;
      items[next]?.focus();
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", onKey);
    window.addEventListener("resize", zu);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", zu);
      window.removeEventListener("scroll", onScroll, true);
    };
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
    const name = `${modelLabel(active.model.remote_id, active.model.label)}${effort ? ` (${effort})` : ""}${active.model.fast ? " ⚡" : ""}`;
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

  const choose = async (id: string | null) => {
    setOpen(false);
    setError(null);
    const result = fn
      ? await commands.llmSetFunctionModel(fn, id, null)
      : id
        ? await commands.llmSetActiveModel(id)
        : null;
    if (!result) return;
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
    active?.connection.kind === "ollama" || (!fn && ollamaLoaded.length > 0);

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
  const ollamaTitle = !ollamaRelevant
    ? undefined
    : ollamaBusy
      ? t("tts.llm.busy")
      : (ollamaError ??
        (ollamaLoaded.length > 0
          ? t("tts.llm.loaded", { models: ollamaLoaded.join(", ") })
          : undefined));
  const fnName = fn ? t(`settings.llm.functions.names.${fn}`) : null;
  const fnTitle = fn
    ? t("footer.functionModel.title", {
        name: fnName,
        model: active
          ? `${active.connection.label} · ${modelLabel(active.model.remote_id, active.model.label)}`
          : t("footer.functionModel.none"),
      })
    : undefined;
  const title =
    error ??
    ollamaTitle ??
    status?.message ??
    budgetTitle ??
    ([fnTitle, spendTitle].filter(Boolean).join("\n") || undefined) ??
    undefined;

  const trigger = (
    <button
      ref={knopfRef}
      type="button"
      onClick={() => setOpen(!open)}
      onContextMenu={(event) => {
        event.preventDefault();
        setOpen(false);
        setContextAt({ x: event.clientX, y: event.clientY });
      }}
      className="lv-status-btn"
      title={title}
      aria-label={fnTitle ?? t("llmSelector.title")}
      aria-haspopup="menu"
      aria-expanded={open}
      data-llm-selector={fn ? undefined : ""}
      data-testid={fn ? `fuss-modell-${fn}` : undefined}
    >
      <span className="relative inline-flex">
        <Sparkle size={14} aria-hidden="true" />
        <div
          className={`absolute -bottom-px -end-px w-1.5 h-1.5 rounded-full ring-1 ring-background ${light()}`}
        />
      </span>
      <span className="lv-status-label max-w-56 truncate">{label()}</span>
      {spend && spend.month.cost > 0 && (
        <span
          className="lv-status-label text-xs tabular-nums text-text/60"
          data-llm-spend
        >
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
  );

  return (
    <div className="relative" ref={ref} data-function-chip={fn}>
      {title ? (
        trigger
      ) : (
        <RoleTip
          role={t("roles.llm.role")}
          name={label()}
          purpose={t("roles.llm.purpose")}
        >
          {trigger}
        </RoleTip>
      )}
      {contextAt && (
        <div
          className="fixed z-50 min-w-48 bg-background border border-mid-gray/20 rounded-lg shadow-lg py-1"
          style={{
            left: contextAt.x,
            bottom: window.innerHeight - contextAt.y,
          }}
          role="menu"
          onMouseDown={(event) => event.stopPropagation()}
          data-llm-context-menu
        >
          <button
            type="button"
            role="menuitem"
            onClick={openProviderSettings}
            className="w-full px-3 py-2 text-start text-sm text-text/80 hover:bg-mid-gray/10"
            data-llm-provider-settings
          >
            {t("llmSelector.providerSettings")}
          </button>
        </div>
      )}
      {open && (
        <div
          className="lv-fuss-menue absolute bottom-full start-0 mb-2 w-72 max-w-[calc(100vw-16px)] max-h-[60vh] overflow-y-auto bg-background border border-mid-gray/20 rounded-lg shadow-lg py-2 z-50"
          style={
            anker
              ? {
                  position: "fixed",
                  left: anker.left,
                  right: "auto",
                  bottom: anker.bottom,
                }
              : undefined
          }
          ref={menueRef}
          role="menu"
          aria-label={fnName ?? undefined}
          data-testid={fn ? `fuss-modell-menue-${fn}` : undefined}
        >
          {selectable.length > 0 && (
            <div className="sticky top-0 z-10 -mt-2 mb-1 bg-background px-3 pb-2 pt-2">
              <input
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    if (query) {
                      e.stopPropagation();
                      setQuery("");
                    } else setOpen(false);
                  } else if (e.key === "Enter") {
                    e.preventDefault();
                    const first = visible.find(
                      ({ model }) => byModel[model.id]?.verdict !== "blocked",
                    );
                    if (first) void choose(first.model.id);
                  }
                }}
                autoFocus
                placeholder={t("llmSelector.search")}
                aria-label={t("llmSelector.search")}
                className="w-full rounded-md border border-mid-gray/30 bg-background px-2 py-1.5 text-sm [@media(pointer:coarse)]:min-h-[44px]"
                data-llm-search
              />
            </div>
          )}
          {fn && (
            <>
              <p className="px-3 pb-1 text-xs text-text/60">
                {t(`settings.llm.functions.hints.${fn}`)}
              </p>
              <button
                type="button"
                role="menuitemradio"
                aria-checked={!own}
                onClick={() => void choose(null)}
                className={`flex w-full min-h-[44px] items-center justify-start px-3 py-2 text-start text-sm text-text/80 transition-colors hover:bg-mid-gray/10 ${
                  !own ? "bg-logo-primary/10 text-logo-primary" : ""
                }`}
                data-function-option=""
              >
                {globalActive
                  ? t("settings.llm.functions.default", {
                      model: modelLabel(
                        globalActive.model.remote_id,
                        globalActive.model.label,
                      ),
                    })
                  : t("settings.llm.functions.defaultNone")}
              </button>
            </>
          )}
          {selectable.length === 0 ? (
            <div className="px-3 py-2 text-sm text-text/60">
              {t("llmSelector.empty")}
            </div>
          ) : visible.length === 0 ? (
            <div className="px-3 py-2 text-sm text-text/60" data-llm-no-match>
              {t("llmSelector.noMatch")}
            </div>
          ) : (
            visible.map(({ model, connection }) => {
              const assessment = byModel[model.id];
              // Gesperrt: sichtbar mit Grund, aber nicht waehlbar.
              const blocked = assessment?.verdict === "blocked";
              return (
                <div
                  key={model.id}
                  role={fn ? "menuitemradio" : "menuitem"}
                  aria-checked={fn ? model.id === checkedId : undefined}
                  tabIndex={blocked ? -1 : 0}
                  aria-disabled={blocked || undefined}
                  data-llm-option={model.id}
                  data-function-option={fn ? model.id : undefined}
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
                    model.id === checkedId
                      ? "bg-logo-primary/10 text-logo-primary"
                      : ""
                  }`}
                >
                  <div className="flex items-center gap-2 text-sm text-text/80">
                    <span className="min-w-0 truncate">
                      {modelLabel(model.remote_id, model.label)}
                    </span>
                    {effortOf(model, connection) && (
                      <span className="text-xs text-text/50" data-llm-effort>
                        ({effortText(effortOf(model, connection))})
                      </span>
                    )}
                    {model.fast && (
                      <span
                        className="text-xs"
                        title={t("settings.llm.model.fastOn")}
                        data-llm-fast
                      >
                        ⚡
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
          <button
            type="button"
            role="menuitem"
            onClick={openProviderSettings}
            className="w-full px-3 py-2 mt-1 border-t border-mid-gray/20 text-start text-sm text-text/70 hover:bg-mid-gray/10"
            data-llm-provider-settings-link
          >
            {t("llmSelector.providerSettings")}
          </button>
        </div>
      )}
    </div>
  );
};

export default LlmSelector;
