import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Play } from "lucide-react";
import {
  commands,
  type LlmDownloadInfo,
  type WorkflowAgentTool,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";
import { Textarea } from "../ui/Textarea";
import { AgentOutcome } from "./AgentViews";
import {
  EXTRACT_KINDS,
  MAX_TOOLS,
  asList,
  parseAgentPreview,
  toggleChoice,
  type AgentAction,
  type AgentPreview,
} from "./agentModel";
import { errorOf } from "./useAutomations";

/** Die Werkzeuge der Politik aendern sich nur mit der App: einmal laden. */
let toolsCache: WorkflowAgentTool[] | null = null;

export function useAgentTools(): WorkflowAgentTool[] | null {
  const [tools, setTools] = useState<WorkflowAgentTool[] | null>(toolsCache);
  useEffect(() => {
    if (toolsCache) return;
    let alive = true;
    void commands.workflowAgentTools().then(
      (r) => {
        if (r.status !== "ok") return;
        toolsCache = r.data;
        if (alive) setTools(r.data);
      },
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, []);
  return tools;
}

const FieldFrame: React.FC<{
  legend: string;
  hint?: string;
  required?: boolean;
  issues: string[];
  testId: string;
  children: React.ReactNode;
}> = ({ legend, hint, required, issues, testId, children }) => {
  const id = useId();
  return (
    <fieldset
      className="min-w-0 space-y-1"
      aria-describedby={hint ? `${id}-hint` : undefined}
      data-testid={testId}
    >
      <legend className="text-sm font-medium">
        {legend}
        {required && (
          <span aria-hidden="true" className="text-status-red">
            {" *"}
          </span>
        )}
      </legend>
      {children}
      {hint && (
        <p id={`${id}-hint`} className="text-xs text-text-muted">
          {hint}
        </p>
      )}
      {issues.length > 0 && (
        <p className="text-xs text-status-red" data-testid={`${testId}-error`}>
          {issues.join(" ")}
        </p>
      )}
    </fieldset>
  );
};

const CheckRow: React.FC<{
  checked: boolean;
  disabled?: boolean;
  label: React.ReactNode;
  detail?: React.ReactNode;
  testId: string;
  onChange: (on: boolean) => void;
}> = ({ checked, disabled, label, detail, testId, onChange }) => (
  <li className="rounded-md border border-mid-gray/25 px-2 py-1.5">
    <label className="flex items-start gap-2 text-sm">
      <input
        type="checkbox"
        className="mt-1"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        data-testid={testId}
      />
      <span className="min-w-0">
        <span className="font-medium">{label}</span>
        {detail && (
          <span className="mt-0.5 block text-xs text-text-muted">{detail}</span>
        )}
      </span>
    </label>
  </li>
);

/**
 * Werkzeug-Whitelist eines `agent.route`-Schritts als Mehrfachauswahl aus dem Katalog der
 * Politik. Zeigt je gewaehltem Werkzeug, welche Felder das Modell fuellt (Schema-Vorschau).
 */
export const ToolsField: React.FC<{
  value: unknown;
  issues: string[];
  testPrefix: string;
  onChange: (next: string[]) => void;
}> = ({ value, issues, testPrefix, onChange }) => {
  const { t } = useTranslation();
  const tools = useAgentTools();
  const chosen = asList(value);
  const names = (tools ?? []).map((x) => x.name);
  const unknown = chosen.filter((n) => !names.includes(n));
  const full = chosen.length >= MAX_TOOLS;
  return (
    <FieldFrame
      legend={t("automations.fields.tools.label")}
      hint={t("automations.fields.tools.hint", { max: MAX_TOOLS })}
      required
      issues={issues}
      testId={`${testPrefix}-tools`}
    >
      {tools === null ? (
        <p className="text-xs text-text-muted">{t("automations.loading")}</p>
      ) : (
        <ul className="space-y-1.5">
          {tools.map((tool) => {
            const on = chosen.includes(tool.name);
            return (
              <CheckRow
                key={tool.name}
                checked={on}
                disabled={!on && full}
                testId={`${testPrefix}-tool-${tool.name}`}
                onChange={(next) =>
                  onChange(toggleChoice(chosen, tool.name, next, names))
                }
                label={
                  <>
                    {tool.name}
                    {tool.sends_mail && (
                      <span className="ms-2 rounded-full bg-amber-500/25 px-2 py-0.5 text-xs font-medium">
                        {t("automations.agent.sendsMail")}
                      </span>
                    )}
                  </>
                }
                detail={
                  <>
                    {tool.description}
                    <span className="block">
                      {t("automations.agent.runsAction", {
                        action: tool.action,
                      })}
                    </span>
                    {on && (
                      <span
                        className="block"
                        data-testid={`${testPrefix}-tool-${tool.name}-schema`}
                      >
                        {t("automations.agent.toolFields", {
                          fields: tool.params
                            .map(
                              (p) =>
                                `${p.key}${p.required ? " *" : ""}${p.kind === "date" ? ` (${t("automations.agent.dateKind")})` : ""}`,
                            )
                            .join(", "),
                        })}
                      </span>
                    )}
                  </>
                }
              />
            );
          })}
          {unknown.map((name) => (
            <CheckRow
              key={name}
              checked
              testId={`${testPrefix}-tool-${name}`}
              onChange={() =>
                onChange(toggleChoice(chosen, name, false, names))
              }
              label={name}
              detail={t("automations.agent.unknownTool")}
            />
          ))}
        </ul>
      )}
    </FieldFrame>
  );
};

/** Welche Listen `agent.extract` zieht (ohne Auswahl: alle). */
export const KindsField: React.FC<{
  value: unknown;
  issues: string[];
  testPrefix: string;
  onChange: (next: string[]) => void;
}> = ({ value, issues, testPrefix, onChange }) => {
  const { t } = useTranslation();
  const chosen = asList(value);
  return (
    <FieldFrame
      legend={t("automations.fields.kinds.label")}
      hint={t("automations.fields.kinds.hint")}
      issues={issues}
      testId={`${testPrefix}-kinds`}
    >
      <ul className="flex flex-wrap gap-2">
        {EXTRACT_KINDS.map((kind) => (
          <CheckRow
            key={kind}
            checked={chosen.includes(kind)}
            testId={`${testPrefix}-kind-${kind}`}
            onChange={(on) =>
              onChange(toggleChoice(chosen, kind, on, EXTRACT_KINDS))
            }
            label={t(`automations.agent.kinds.${kind}`)}
          />
        ))}
      </ul>
    </FieldFrame>
  );
};

/** Router-Modell: ein geladenes lokales Modell oder der Standard. */
export const ModelField: React.FC<{
  value: unknown;
  issues: string[];
  testPrefix: string;
  onChange: (next: string | undefined) => void;
}> = ({ value, issues, testPrefix, onChange }) => {
  const { t } = useTranslation();
  const id = useId();
  const [models, setModels] = useState<LlmDownloadInfo[] | null>(null);
  useEffect(() => {
    let alive = true;
    void commands.llmLocalList().then(
      (list) => {
        if (alive) setModels(list);
      },
      () => {
        if (alive) setModels([]);
      },
    );
    return () => {
      alive = false;
    };
  }, []);
  const current = typeof value === "string" && value ? value : null;
  const options = (models ?? [])
    .filter((m) => m.kind === "model" && m.is_downloaded)
    .map((m) => ({ value: m.id, label: m.name }));
  if (current && !options.some((o) => o.value === current)) {
    options.unshift({
      value: current,
      label: t("automations.agent.modelNotLoaded", { id: current }),
    });
  }
  const label = t("automations.fields.model.label");
  return (
    <div
      className="min-w-0 space-y-1"
      data-field="model"
      data-testid={`${testPrefix}-model`}
    >
      <label htmlFor={id} className="block text-sm font-medium">
        {label}
      </label>
      <Select
        value={current}
        options={options}
        isClearable
        placeholder={t("automations.agent.modelDefault")}
        ariaLabel={label}
        onChange={(v) => onChange(v ?? undefined)}
        menuPortal
      />
      <p id={id} className="text-xs text-text-muted">
        {t("automations.fields.model.hint")}
      </p>
      {issues.length > 0 && (
        <p className="text-xs text-status-red">{issues.join(" ")}</p>
      )}
    </div>
  );
};

/**
 * „Mit Beispieltext ausprobieren“: das Modell entscheidet fuer den Entwurf aus dem Editor, es
 * geschieht nichts (kein Lauf, keine Freigabe, keine Herkunft). Die Rechnung macht das Backend.
 */
export const AgentPreviewPanel: React.FC<{
  action: AgentAction;
  definitionJson: string;
  workflowId: string | null;
  stepId: string;
  testPrefix: string;
}> = ({ action, definitionJson, workflowId, stepId, testPrefix }) => {
  const { t } = useTranslation();
  const uid = useId();
  const kind = action === "agent.route" ? "route" : "extract";
  const [sample, setSample] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<AgentPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Nur die jüngste Anfrage zaehlt: eine spaete Antwort ueberschreibt keine neuere.
  const ticket = useRef(0);
  const needsSample = kind === "extract";

  const run = async () => {
    const mine = ++ticket.current;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await commands.workflowAgentPreview(
        workflowId,
        definitionJson,
        stepId,
        sample.trim() ? sample : null,
      );
      if (mine !== ticket.current) return;
      if (r.status === "ok") {
        const parsed = parseAgentPreview(r.data, kind);
        if (parsed) setResult(parsed);
        else setError(t("automations.agent.previewUnreadable"));
      } else {
        setError(r.error);
      }
    } catch (e) {
      if (mine === ticket.current)
        setError(errorOf(e) || t("automations.errors.generic"));
    }
    if (mine === ticket.current) setBusy(false);
  };

  return (
    <section
      className="space-y-2 rounded-md border border-dashed border-mid-gray/50 p-3"
      aria-labelledby={`${uid}-title`}
      data-testid={`${testPrefix}-agent-preview`}
    >
      <h4 id={`${uid}-title`} className="text-sm font-semibold">
        {t("automations.agent.previewTitle")}
      </h4>
      <p className="text-xs text-text-muted">
        {t(
          kind === "route"
            ? "automations.agent.previewHintRoute"
            : "automations.agent.previewHintExtract",
        )}
      </p>
      <div className="space-y-1">
        <label htmlFor={`${uid}-sample`} className="block text-sm font-medium">
          {t("automations.agent.sample")}
          {needsSample && (
            <span aria-hidden="true" className="text-status-red">
              {" *"}
            </span>
          )}
        </label>
        <Textarea
          id={`${uid}-sample`}
          className="w-full"
          variant="compact"
          value={sample}
          placeholder={t("automations.agent.samplePlaceholder")}
          onChange={(e) => setSample(e.target.value)}
          data-testid={`${testPrefix}-agent-sample`}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || (needsSample && !sample.trim())}
          onClick={() => void run()}
          data-testid={`${testPrefix}-agent-try`}
        >
          <Play size={14} aria-hidden="true" />
          {busy ? t("automations.agent.trying") : t("automations.agent.try")}
        </Button>
        <span
          className="rounded-full bg-mid-gray/20 px-2 py-0.5 text-xs font-medium"
          data-testid={`${testPrefix}-agent-nowrite`}
        >
          {t("automations.agent.noEffect")}
        </span>
      </div>
      <div aria-live="polite" className="space-y-2">
        {error && (
          <p
            className="break-words text-sm text-status-red"
            role="alert"
            data-testid={`${testPrefix}-agent-error`}
          >
            {error}
          </p>
        )}
        {result?.busy && (
          <p
            className="rounded-md bg-amber-500/20 px-2 py-1 text-sm"
            data-testid={`${testPrefix}-agent-busy`}
          >
            {t("automations.agent.busy", {
              seconds: Math.max(1, Math.round(result.busy.retryAfterMs / 1000)),
              reason: result.busy.message,
            })}
          </p>
        )}
        {result?.skipped && (
          <p
            className="rounded-md bg-mid-gray/15 px-2 py-1 text-sm"
            data-testid={`${testPrefix}-agent-skipped`}
          >
            {result.skipped.text}
          </p>
        )}
        {result && !result.busy && !result.skipped && (
          <AgentOutcome result={result} testId={`${testPrefix}-agent-result`} />
        )}
      </div>
    </section>
  );
};
