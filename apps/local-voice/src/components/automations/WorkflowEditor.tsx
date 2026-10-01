import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, ListChecks, Save } from "lucide-react";
import {
  commands,
  type IntegrationView,
  type WorkflowCatalog,
  type WorkflowItem,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import { Textarea } from "../ui/Textarea";
import { FieldInput } from "./FieldInput";
import { StepEditor, actionTitle } from "./StepEditor";
import { VariablesEditor } from "./VariablesEditor";
import { errorOf } from "./useAutomations";
import {
  cloneDef,
  isBlank,
  issuesAt,
  moveStep,
  nextStepId,
  prettyPath,
  retypeTrigger,
  sameDef,
  serializeDef,
  withParam,
  type Def,
  type Issue,
} from "./model";

interface WorkflowEditorProps {
  catalog: WorkflowCatalog;
  integrations: IntegrationView[];
  /** `null`: ein neuer Ablauf. */
  workflowId: string | null;
  initial: Def;
  onSaved: (item: WorkflowItem) => void;
  onBack: () => void;
  onPlan: (
    definitionJson: string,
    workflowId: string | null,
    name: string,
  ) => void;
}

const VALIDATE_DELAY_MS = 350;

/**
 * Formular-Editor: Ausloeser, Variablen, Schritte. Alles, was ein Feld ist, kommt aus dem
 * Katalog des Backends; die Pruefung ebenso (`workflow_validate`, dieselbe wie beim
 * Speichern) und steht mit ihrem JSON-Zeiger am Feld und gesammelt oben. Der Entwurf wird
 * nie ohne Klick auf „Speichern“ geschrieben.
 */
export const WorkflowEditor: React.FC<WorkflowEditorProps> = ({
  catalog,
  integrations,
  workflowId,
  initial,
  onSaved,
  onBack,
  onPlan,
}) => {
  const { t } = useTranslation();
  const uid = useId();
  const [def, setDef] = useState<Def>(() => cloneDef(initial));
  const [issues, setIssues] = useState<Issue[]>([]);
  const [touched, setTouched] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [saved, setSaved] = useState(false);
  const base = useRef(cloneDef(initial));
  const dirty = !sameDef(def, base.current);
  const json = useMemo(() => serializeDef(def), [def]);

  const update = (next: Def) => {
    setTouched(true);
    setSaved(false);
    setDef(next);
  };

  // Pruefung beim Tippen: dieselbe Rechnung wie beim Speichern.
  useEffect(() => {
    let alive = true;
    const timer = window.setTimeout(() => {
      void commands.workflowValidate(json).then(
        (r) => {
          if (alive && r.status === "ok") setIssues(r.data);
        },
        () => undefined,
      );
    }, VALIDATE_DELAY_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [json]);

  const showIssues = touched || !isBlank(initial);
  const shown = showIssues ? issues : [];
  const triggerSpec = catalog.triggers.find((x) => x.id === def.trigger.type);

  const save = async () => {
    setTouched(true);
    setSaving(true);
    setSaveError(null);
    try {
      const r = await commands.workflowSave(workflowId, json);
      if (r.status === "error") {
        setSaveError(r.error);
      } else if (r.data.workflow) {
        base.current = cloneDef(def);
        setSaved(true);
        setIssues([]);
        onSaved(r.data.workflow);
      } else {
        setIssues(r.data.issues);
        setSaveError(
          t("automations.editor.notSaved", { count: r.data.issues.length }),
        );
      }
    } catch (e) {
      setSaveError(errorOf(e) || t("automations.errors.generic"));
    }
    setSaving(false);
  };

  const back = () => {
    if (dirty && !saved) setConfirmLeave(true);
    else onBack();
  };

  const addStep = (action: string) => {
    update({
      ...def,
      steps: [
        ...def.steps,
        { id: nextStepId(def, action), action, params: {} },
      ],
    });
  };

  return (
    <div className="space-y-5" data-testid="workflow-editor">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={back}
          data-testid="editor-back"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          {t("automations.back")}
        </Button>
        <h2 className="min-w-0 break-words text-base font-semibold">
          {workflowId
            ? t("automations.editor.titleEdit")
            : t("automations.editor.titleNew")}
        </h2>
      </div>

      {confirmLeave && (
        <div
          className="flex flex-wrap items-center gap-2 rounded-lg border border-logo-primary bg-logo-primary/15 px-3 py-2"
          role="alert"
          data-testid="editor-leave-confirm"
        >
          <span className="text-sm">{t("automations.editor.discardAsk")}</span>
          <Button size="sm" onClick={onBack} data-testid="editor-discard">
            {t("automations.editor.discard")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setConfirmLeave(false)}
          >
            {t("automations.editor.keepEditing")}
          </Button>
        </div>
      )}

      <section className="space-y-3" aria-labelledby={`${uid}-gen`}>
        <h3 id={`${uid}-gen`} className="text-sm font-semibold">
          {t("automations.editor.general")}
        </h3>
        <div className="space-y-1">
          <label htmlFor={`${uid}-name`} className="text-sm font-medium">
            {t("automations.editor.name")}
            <span aria-hidden="true" className="text-status-red">
              {" *"}
            </span>
          </label>
          <Input
            id={`${uid}-name`}
            className="w-full"
            value={def.name}
            aria-invalid={issuesAt(shown, "/name").length > 0 || undefined}
            onChange={(e) => update({ ...def, name: e.target.value })}
            data-testid="editor-name"
          />
          {issuesAt(shown, "/name").length > 0 && (
            <p className="text-xs text-status-red">
              {issuesAt(shown, "/name").join(" ")}
            </p>
          )}
        </div>
        <div className="space-y-1">
          <label htmlFor={`${uid}-desc`} className="text-sm font-medium">
            {t("automations.editor.description")}
          </label>
          <Textarea
            id={`${uid}-desc`}
            variant="compact"
            className="w-full"
            value={def.description ?? ""}
            onChange={(e) =>
              update({ ...def, description: e.target.value || undefined })
            }
            data-testid="editor-description"
          />
        </div>
      </section>

      <section className="space-y-3" aria-labelledby={`${uid}-trg`}>
        <h3 id={`${uid}-trg`} className="text-sm font-semibold">
          {t("automations.editor.trigger")}
        </h3>
        <div className="space-y-1">
          <span className="block text-sm font-medium">
            {t("automations.editor.triggerType")}
          </span>
          <Select
            value={def.trigger.type}
            options={catalog.triggers.map((x) => ({
              value: x.id,
              label: t(`automations.triggers.${x.id}`, {
                defaultValue: x.title,
              }),
            }))}
            ariaLabel={t("automations.editor.triggerType")}
            onChange={(v) => {
              if (v && v !== def.trigger.type)
                update({ ...def, trigger: retypeTrigger(v) });
            }}
            menuPortal
          />
          {issuesAt(shown, "/trigger/type").length > 0 && (
            <p className="text-xs text-status-red">
              {issuesAt(shown, "/trigger/type").join(" ")}
            </p>
          )}
          {triggerSpec && (
            <p className="text-xs text-text-muted" data-testid="trigger-hint">
              {triggerSpec.automatic
                ? t("automations.editor.triggerAuto")
                : t("automations.editor.triggerManual")}
            </p>
          )}
        </div>
        {triggerSpec && triggerSpec.fields.length > 0 && (
          <div className="grid gap-3 sm:grid-cols-2">
            {triggerSpec.fields.map((f) => (
              <FieldInput
                key={`${triggerSpec.id}-${f.name}`}
                spec={f}
                value={def.trigger[f.name]}
                onChange={(v) =>
                  update({
                    ...def,
                    trigger: withParam(
                      def.trigger,
                      f.name,
                      v,
                    ) as Def["trigger"],
                  })
                }
                issues={issuesAt(shown, `/trigger/${f.name}`)}
                integrations={integrations}
                testPrefix="trigger"
              />
            ))}
          </div>
        )}
        {triggerSpec && triggerSpec.provides.length > 0 && (
          <p className="text-xs text-text-muted">
            {t("automations.editor.provides", {
              fields: triggerSpec.provides
                .map((p) => `trigger.${p}`)
                .join(", "),
            })}
          </p>
        )}
      </section>

      <section className="space-y-3" aria-labelledby={`${uid}-var`}>
        <h3 id={`${uid}-var`} className="text-sm font-semibold">
          {t("automations.editor.variables")}
        </h3>
        <VariablesEditor
          variables={def.variables ?? {}}
          issues={shown}
          onChange={(variables) => update({ ...def, variables })}
        />
      </section>

      <section className="space-y-3" aria-labelledby={`${uid}-stp`}>
        <h3 id={`${uid}-stp`} className="text-sm font-semibold">
          {t("automations.editor.steps")}
        </h3>
        {def.steps.length === 0 && (
          <p className="text-sm text-text-muted" data-testid="steps-empty">
            {t("automations.editor.noSteps")}
          </p>
        )}
        <ol className="space-y-3">
          {def.steps.map((step, i) => (
            <StepEditor
              key={i}
              index={i}
              count={def.steps.length}
              step={step}
              catalog={catalog}
              integrations={integrations}
              issues={shown}
              onChange={(s) =>
                update({
                  ...def,
                  steps: def.steps.map((x, j) => (j === i ? s : x)),
                })
              }
              onMove={(d) =>
                update({ ...def, steps: moveStep(def.steps, i, d) })
              }
              onRemove={() =>
                update({ ...def, steps: def.steps.filter((_, j) => j !== i) })
              }
            />
          ))}
        </ol>
        <div className="flex flex-wrap items-end gap-2">
          <div className="min-w-[14rem] flex-1 space-y-1">
            <span className="block text-sm font-medium">
              {t("automations.editor.addStep")}
            </span>
            <Select
              value={null}
              options={catalog.actions.map((a) => ({
                value: a.id,
                label: actionTitle(t, a, a.id),
              }))}
              placeholder={t("automations.editor.addStepPlaceholder")}
              ariaLabel={t("automations.editor.addStep")}
              onChange={(v) => v && addStep(v)}
              menuPortal
            />
          </div>
        </div>
        {issuesAt(shown, "/steps").length > 0 && (
          <p className="text-xs text-status-red">
            {issuesAt(shown, "/steps").join(" ")}
          </p>
        )}
      </section>

      <section
        className="sticky bottom-0 space-y-2 border-t border-mid-gray/30 bg-background py-3"
        aria-label={t("automations.editor.actions")}
      >
        {shown.length > 0 && (
          <div data-testid="editor-issues" role="status" className="space-y-1">
            <p className="text-sm font-medium text-status-red">
              {t("automations.editor.issues", { count: shown.length })}
            </p>
            <ul className="max-h-28 space-y-0.5 overflow-auto text-xs">
              {shown.map((i) => (
                <li key={`${i.path}:${i.message}`} data-testid="editor-issue">
                  <code className="me-1 rounded bg-mid-gray/20 px-1">
                    {i.path || "/"}
                  </code>
                  <span className="text-text-muted">
                    {prettyPath(i.path, def.steps)}
                  </span>{" "}
                  {i.message}
                </li>
              ))}
            </ul>
          </div>
        )}
        {saveError && (
          <p
            className="text-sm text-status-red"
            role="alert"
            data-testid="editor-save-error"
          >
            {saveError}
          </p>
        )}
        {saved && (
          <p className="text-sm" role="status" data-testid="editor-saved">
            {t("automations.editor.saved")}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={saving || !def.name.trim()}
            onClick={() => void save()}
            data-testid="editor-save"
          >
            <Save size={14} aria-hidden="true" />
            {t("automations.editor.save")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onPlan(json, workflowId, def.name)}
            data-testid="editor-plan"
          >
            <ListChecks size={14} aria-hidden="true" />
            {t("automations.editor.plan")}
          </Button>
        </div>
      </section>
    </div>
  );
};
