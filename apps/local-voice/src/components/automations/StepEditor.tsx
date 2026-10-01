import React, { useId } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp, Trash2 } from "lucide-react";
import type {
  IntegrationView,
  WorkflowActionSpec,
  WorkflowCatalog,
  WorkflowFieldSpec,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import {
  AgentPreviewPanel,
  KindsField,
  ModelField,
  ToolsField,
  useAgentTools,
} from "./AgentFields";
import { asList, isAgentAction } from "./agentModel";
import { FieldInput } from "./FieldInput";
import {
  isValidIdent,
  issuesAt,
  textToInt,
  withParam,
  type Issue,
  type StepDef,
} from "./model";

interface StepEditorProps {
  index: number;
  count: number;
  step: StepDef;
  catalog: WorkflowCatalog;
  integrations: IntegrationView[];
  issues: Issue[];
  /** Der ganze Entwurf als JSON-Text und die Kennung des Ablaufs (`null`: neu): fuer die Vorschau der KI-Schritte. */
  definitionJson?: string;
  workflowId?: string | null;
  onChange: (step: StepDef) => void;
  onMove: (delta: -1 | 1) => void;
  onRemove: () => void;
}

/** Felder, die bei KI-Schritten die ganze Breite brauchen. */
const WIDE_AGENT_FIELDS = new Set(["task", "context", "tools", "kinds"]);

export const actionTitle = (
  t: (key: string, options?: Record<string, unknown>) => string,
  spec: WorkflowActionSpec | undefined,
  id: string,
): string =>
  t(`automations.actions.${id}`, {
    defaultValue: spec?.title ?? id,
  });

/**
 * Ein Schritt des Ablaufs: Baustein, Kennung, Bedingung, die Felder des Bausteins aus dem
 * Katalog und die Fehlerregel. Befunde der Pruefung stehen am Feld, zu dem ihr JSON-Zeiger
 * gehoert (`/steps/<i>/params/<feld>`).
 */
export const StepEditor: React.FC<StepEditorProps> = ({
  index,
  count,
  step,
  catalog,
  integrations,
  issues,
  definitionJson,
  workflowId = null,
  onChange,
  onMove,
  onRemove,
}) => {
  const { t } = useTranslation();
  const uid = useId();
  const agentTools = useAgentTools();
  const spec = catalog.actions.find((a) => a.id === step.action);
  const base = `/steps/${index}`;
  const prefix = `step-${index}`;
  const idIssues = issuesAt(issues, `${base}/id`);
  const whenIssues = issuesAt(issues, `${base}/when`);
  const actionIssues = issuesAt(issues, `${base}/action`);
  const idInvalid = idIssues.length > 0 || !isValidIdent(step.id);
  const fieldNames = new Set((spec?.fields ?? []).map((f) => f.name));
  // Befunde zum Schritt, die kein Feld zeigt (unbekanntes Feld, Pruefung des Bausteins).
  const rest = issues.filter(
    (i) =>
      (i.path === `${base}/params` || i.path === base) &&
      !idIssues.includes(i.message),
  );
  const unknownParams = issues.filter((i) => {
    if (!i.path.startsWith(`${base}/params/`)) return false;
    const name = i.path.slice(`${base}/params/`.length).split("/")[0];
    return !fieldNames.has(name);
  });

  const setParam = (name: string, value: unknown) =>
    onChange({ ...step, params: withParam(step.params, name, value) });

  // KI-Schritte (C5): Felder mit eigener Bedienung. Die Empfaengerregel gilt nur mit einem Werkzeug,
  // das Mail sendet; wer es abwaehlt, verliert Regel und Liste (sonst lehnt die Pruefung ab).
  const agent = isAgentAction(step.action) ? step.action : null;
  const chosenTools = asList(step.params?.tools);
  const sendsMail = (name: string) =>
    agentTools?.find((x) => x.name === name)?.sends_mail ??
    name === "send_mail";
  const mailChosen = chosenTools.some(sendsMail);
  const setTools = (next: string[]) => {
    let params = withParam(step.params, "tools", next);
    if (!next.some(sendsMail)) {
      params = withParam(withParam(params, "recipients", undefined), "list", undefined);
    }
    onChange({ ...step, params });
  };
  const setRecipients = (value: unknown) => {
    let params = withParam(step.params, "recipients", value);
    if (value !== "list") params = withParam(params, "list", undefined);
    onChange({ ...step, params });
  };

  const renderField = (f: WorkflowFieldSpec): React.ReactNode => {
    const fieldIssues = issuesAt(issues, `${base}/params/${f.name}`);
    if (agent === "agent.route") {
      if (f.name === "tools")
        return (
          <ToolsField
            value={step.params?.tools}
            issues={fieldIssues}
            testPrefix={prefix}
            onChange={setTools}
          />
        );
      if (f.name === "recipients" && !mailChosen) return null;
      if (f.name === "list" && !(mailChosen && step.params?.recipients === "list"))
        return null;
      if (f.name === "model")
        return (
          <ModelField
            value={step.params?.model}
            issues={fieldIssues}
            testPrefix={prefix}
            onChange={(v) => setParam("model", v)}
          />
        );
    }
    if (agent === "agent.extract" && f.name === "kinds")
      return (
        <KindsField
          value={step.params?.kinds}
          issues={fieldIssues}
          testPrefix={prefix}
          onChange={(v) => setParam("kinds", v)}
        />
      );
    return (
      <FieldInput
        spec={f}
        value={step.params?.[f.name]}
        onChange={(v) =>
          agent === "agent.route" && f.name === "recipients"
            ? setRecipients(v)
            : setParam(f.name, v)
        }
        issues={fieldIssues}
        integrations={integrations}
        testPrefix={prefix}
      />
    );
  };

  return (
    <li
      className="space-y-3 rounded-lg border border-mid-gray/30 p-3"
      data-testid="step-editor"
      data-step-index={index}
      data-step-id={step.id}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="min-w-0 break-words text-sm font-semibold">
          {t("automations.editor.stepTitle", {
            n: index + 1,
            title: actionTitle(t, spec, step.action),
          })}
        </h3>
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="sm"
            disabled={index === 0}
            onClick={() => onMove(-1)}
            aria-label={t("automations.editor.moveUp", { n: index + 1 })}
            data-testid={`${prefix}-up`}
          >
            <ArrowUp size={14} aria-hidden="true" />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={index === count - 1}
            onClick={() => onMove(1)}
            aria-label={t("automations.editor.moveDown", { n: index + 1 })}
            data-testid={`${prefix}-down`}
          >
            <ArrowDown size={14} aria-hidden="true" />
          </Button>
          <Button
            variant="danger-ghost"
            size="sm"
            onClick={onRemove}
            aria-label={t("automations.editor.removeStep", { n: index + 1 })}
            data-testid={`${prefix}-remove`}
          >
            <Trash2 size={14} aria-hidden="true" />
          </Button>
        </div>
      </div>

      {spec && (
        <p className="text-xs text-text-muted" data-testid={`${prefix}-effect`}>
          {t(`automations.effectKinds.${spec.effect}`, {
            defaultValue: spec.effect,
          })}
          {spec.heavy_label
            ? ` · ${t("automations.editor.heavy", { what: spec.heavy_label })}`
            : ""}
          {spec.capability
            ? ` · ${t("automations.editor.needsRight", { right: spec.capability })}`
            : ""}
        </p>
      )}

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="min-w-0 space-y-1">
          <label
            htmlFor={`${uid}-action`}
            className="block text-sm font-medium"
          >
            {t("automations.editor.action")}
          </label>
          <Select
            value={step.action}
            options={catalog.actions.map((a) => ({
              value: a.id,
              label: actionTitle(t, a, a.id),
            }))}
            ariaLabel={t("automations.editor.action")}
            onChange={(v) => {
              if (v && v !== step.action) {
                onChange({ ...step, action: v, params: {} });
              }
            }}
            menuPortal
          />
          {actionIssues.length > 0 && (
            <p className="text-xs text-status-red">{actionIssues.join(" ")}</p>
          )}
        </div>
        <div className="min-w-0 space-y-1">
          <label htmlFor={`${uid}-id`} className="block text-sm font-medium">
            {t("automations.editor.stepId")}
          </label>
          <Input
            id={`${uid}-id`}
            className="w-full"
            value={step.id}
            aria-invalid={idInvalid || undefined}
            onChange={(e) => onChange({ ...step, id: e.target.value })}
            data-testid={`${prefix}-id`}
          />
          <p className="text-xs text-text-muted">
            {idIssues.length > 0
              ? idIssues.join(" ")
              : !isValidIdent(step.id)
                ? t("automations.editor.stepIdRule")
                : t("automations.editor.stepIdHint", { id: step.id })}
          </p>
        </div>
        <div className="min-w-0 space-y-1 sm:col-span-2">
          <label htmlFor={`${uid}-label`} className="block text-sm font-medium">
            {t("automations.editor.stepLabel")}
          </label>
          <Input
            id={`${uid}-label`}
            className="w-full"
            value={step.label ?? ""}
            onChange={(e) =>
              onChange({ ...step, label: e.target.value || undefined })
            }
            data-testid={`${prefix}-label`}
          />
        </div>
        <div className="min-w-0 space-y-1 sm:col-span-2">
          <label htmlFor={`${uid}-when`} className="block text-sm font-medium">
            {t("automations.editor.when")}
          </label>
          <Input
            id={`${uid}-when`}
            className="w-full"
            value={step.when ?? ""}
            aria-invalid={whenIssues.length > 0 || undefined}
            aria-describedby={`${uid}-when-hint`}
            placeholder="{{trigger.external_attendees}} > 0"
            onChange={(e) =>
              onChange({ ...step, when: e.target.value || undefined })
            }
            data-testid={`${prefix}-when`}
          />
          <p
            id={`${uid}-when-hint`}
            className={`text-xs ${whenIssues.length > 0 ? "text-status-red" : "text-text-muted"}`}
            data-testid={
              whenIssues.length > 0 ? `${prefix}-when-error` : undefined
            }
          >
            {whenIssues.length > 0
              ? whenIssues.join(" ")
              : t("automations.editor.whenHint")}
          </p>
        </div>

        {(spec?.fields ?? []).map((f) => {
          const field = renderField(f);
          if (field === null) return null;
          const wide =
            ["body", "text", "content", "subject", "path", "name"].includes(
              f.name,
            ) ||
            (agent !== null && WIDE_AGENT_FIELDS.has(f.name));
          return (
            <div key={f.name} className={wide ? "sm:col-span-2" : ""}>
              {field}
            </div>
          );
        })}
      </div>

      {agent && definitionJson !== undefined && (
        <AgentPreviewPanel
          action={agent}
          definitionJson={definitionJson}
          workflowId={workflowId}
          stepId={step.id}
          testPrefix={prefix}
        />
      )}

      {(rest.length > 0 || unknownParams.length > 0) && (
        <ul className="space-y-0.5 text-xs text-status-red">
          {[...rest, ...unknownParams].map((i) => (
            <li key={`${i.path}:${i.message}`}>{i.message}</li>
          ))}
        </ul>
      )}

      <details className="text-sm">
        <summary className="cursor-pointer text-xs font-medium text-text-muted">
          {t("automations.editor.errorRules")}
        </summary>
        <div className="mt-2 grid gap-3 sm:grid-cols-3">
          <div className="min-w-0 space-y-1">
            <span className="block text-sm font-medium">
              {t("automations.editor.onError")}
            </span>
            <Select
              value={step.on_error ?? "fail"}
              options={[
                { value: "fail", label: t("automations.editor.onErrorFail") },
                {
                  value: "continue",
                  label: t("automations.editor.onErrorContinue"),
                },
              ]}
              ariaLabel={t("automations.editor.onError")}
              onChange={(v) =>
                onChange({
                  ...step,
                  on_error: v === "continue" ? "continue" : undefined,
                })
              }
              menuPortal
            />
          </div>
          <div className="min-w-0 space-y-1">
            <label htmlFor={`${uid}-att`} className="block text-sm font-medium">
              {t("automations.editor.maxAttempts")}
            </label>
            <Input
              id={`${uid}-att`}
              className="w-full"
              type="number"
              min={1}
              max={10}
              value={step.retry?.max_attempts ?? ""}
              onChange={(e) =>
                onChange({
                  ...step,
                  retry: retryWith(step, {
                    max_attempts: textToInt(e.target.value),
                  }),
                })
              }
              data-testid={`${prefix}-attempts`}
            />
          </div>
          <div className="min-w-0 space-y-1">
            <label
              htmlFor={`${uid}-back`}
              className="block text-sm font-medium"
            >
              {t("automations.editor.backoff")}
            </label>
            <Input
              id={`${uid}-back`}
              className="w-full"
              type="number"
              min={100}
              step={100}
              value={step.retry?.backoff_ms ?? ""}
              onChange={(e) =>
                onChange({
                  ...step,
                  retry: retryWith(step, {
                    backoff_ms: textToInt(e.target.value),
                  }),
                })
              }
              data-testid={`${prefix}-backoff`}
            />
          </div>
        </div>
      </details>
    </li>
  );
};

/** Wiederholungsregel mit geaendertem Feld; ohne jede Angabe entfaellt sie. */
const retryWith = (
  step: StepDef,
  change: { max_attempts?: number; backoff_ms?: number },
): StepDef["retry"] => {
  const next = { ...(step.retry ?? {}), ...change };
  if (next.max_attempts === undefined) delete next.max_attempts;
  if (next.backoff_ms === undefined) delete next.backoff_ms;
  return Object.keys(next).length > 0 ? next : undefined;
};
