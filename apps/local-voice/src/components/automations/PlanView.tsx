import React from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { StateBadge } from "./StateBadge";
import { prettyJson } from "./model";

/** Der Plan, wie ihn `workflow_plan` liefert (`lva-workflow-plan@1`). Nur gelesene Felder. */
export interface Plan {
  valid: boolean;
  issues?: { path: string; message: string }[];
  trigger?: { type: string };
  trigger_sample?: boolean;
  variables_without_default?: string[];
  steps?: PlanStep[];
  summary?: {
    steps: number;
    planned: number;
    skipped: number;
    invalid: number;
    allowed: number;
    needs_approval: number;
    denied: number;
    not_required: number;
    heavy: number;
    external_effects: number;
    would_run_without_intervention: boolean;
  };
}

interface PlanStep {
  index: number;
  id: string;
  action: string;
  title: string;
  label?: string | null;
  status: string;
  condition?: { expression?: string | null; result: string; message?: string };
  params?: unknown;
  unresolved?: string[];
  param_errors?: string[];
  effect?: string;
  effect_kind?: string;
  heavy?: { label: string; ram_mb: number } | null;
  permission?: {
    required: boolean;
    result: string;
    integration?: string;
    capability?: string;
    mode?: string;
    message?: string;
    preview?: string;
  };
}

export const parsePlan = (text: string): Plan | null => {
  try {
    const v = JSON.parse(text) as Plan;
    return v && typeof v === "object" ? v : null;
  } catch {
    return null;
  }
};

interface PlanViewProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** `null`: wird berechnet. */
  plan: Plan | null;
  failed: boolean;
  name: string;
}

/** Tonfall der Rechte-Anzeige je Ergebnis. */
const permTone = (result: string) =>
  result === "denied" || result === "invalid"
    ? "red"
    : result === "needs_approval"
      ? "amber"
      : result === "allowed"
        ? "green"
        : "gray";

/**
 * Trockenlauf: was WUERDE der Ablauf tun? Jeder Schritt mit Bedingung, geplanter Wirkung und
 * dem Ergebnis der Rechtepruefung. Die Daten des Ausloesers sind Beispieldaten; geschrieben
 * oder gesendet wird nichts.
 */
export const PlanView: React.FC<PlanViewProps> = ({
  open,
  onOpenChange,
  plan,
  failed,
  name,
}) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("automations.plan.title", { name })}
      description={t("automations.plan.description")}
      closeLabel={t("automations.close")}
      className="max-w-3xl"
      footer={
        <Button
          size="sm"
          variant="secondary"
          onClick={() => onOpenChange(false)}
        >
          {t("automations.close")}
        </Button>
      }
    >
      <div className="space-y-3" data-testid="plan-view">
        {failed && (
          <p className="text-sm text-status-red" role="alert">
            {t("automations.plan.failed")}
          </p>
        )}
        {!failed && plan === null && (
          <p className="text-sm text-text-muted">
            {t("automations.plan.loading")}
          </p>
        )}
        {plan && !plan.valid && (
          <div role="alert" data-testid="plan-invalid" className="space-y-1">
            <p className="text-sm font-medium">
              {t("automations.plan.invalid")}
            </p>
            <ul className="space-y-0.5 text-sm text-status-red">
              {(plan.issues ?? []).map((i) => (
                <li key={`${i.path}:${i.message}`}>
                  {i.path ? <code className="text-xs">{i.path}</code> : null}{" "}
                  {i.message}
                </li>
              ))}
            </ul>
          </div>
        )}
        {plan?.valid && plan.summary && (
          <>
            <p
              className={`rounded-lg px-3 py-2 text-sm ${
                plan.summary.would_run_without_intervention
                  ? "bg-green-500/10"
                  : "bg-amber-500/10"
              }`}
              data-testid="plan-summary"
              data-ok={plan.summary.would_run_without_intervention}
            >
              {t("automations.plan.summary", {
                steps: plan.summary.steps,
                allowed: plan.summary.allowed,
                ask: plan.summary.needs_approval,
                denied: plan.summary.denied,
                invalid: plan.summary.invalid,
              })}
              {plan.summary.would_run_without_intervention
                ? ` ${t("automations.plan.runsThrough")}`
                : ` ${t("automations.plan.needsWork")}`}
            </p>
            <p className="text-xs text-text-muted">
              {t("automations.plan.sample")}
            </p>
            {(plan.variables_without_default ?? []).length > 0 && (
              <p className="text-xs text-text-muted">
                {t("automations.plan.varsWithout", {
                  names: (plan.variables_without_default ?? []).join(", "),
                })}
              </p>
            )}
          </>
        )}
        <ol className="space-y-2">
          {(plan?.steps ?? []).map((s) => {
            const perm = s.permission;
            return (
              <li
                key={s.id}
                className="space-y-1 rounded-lg border border-mid-gray/30 p-3"
                data-testid="plan-step"
                data-step-id={s.id}
                data-status={s.status}
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="min-w-0 break-words text-sm font-medium">
                    {s.index + 1}. {s.label || s.title}
                    <span className="ms-2 text-xs font-normal text-text-muted">
                      {s.action}
                    </span>
                  </p>
                  <StateBadge
                    state={s.status}
                    label={t(`automations.plan.status.${s.status}`, {
                      defaultValue: s.status,
                    })}
                  />
                </div>
                {s.effect && <p className="text-sm">{s.effect}</p>}
                {s.condition?.expression && (
                  <p className="text-xs text-text-muted">
                    {t("automations.plan.condition", {
                      expr: s.condition.expression,
                      result: t(`automations.plan.cond.${s.condition.result}`, {
                        defaultValue: s.condition.result,
                      }),
                    })}
                    {s.condition.message ? ` ${s.condition.message}` : ""}
                  </p>
                )}
                {perm && perm.required && (
                  <p className="flex flex-wrap items-center gap-2 text-xs">
                    <StateBadge
                      state={perm.result}
                      tone={permTone(perm.result)}
                      label={t(`automations.plan.perm.${perm.result}`, {
                        defaultValue: perm.result,
                      })}
                      testId="plan-permission"
                    />
                    <span className="text-text-muted">
                      {perm.integration ?? ""}
                      {perm.capability ? ` · ${perm.capability}` : ""}
                      {perm.message ? ` · ${perm.message}` : ""}
                    </span>
                  </p>
                )}
                {perm?.preview && (
                  <pre className="max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs">
                    {perm.preview}
                  </pre>
                )}
                {s.heavy && (
                  <p className="text-xs text-text-muted">
                    {t("automations.plan.heavy", {
                      what: s.heavy.label,
                      ram: Math.round(s.heavy.ram_mb / 102.4) / 10,
                    })}
                  </p>
                )}
                {(s.param_errors ?? []).length > 0 && (
                  <p className="text-xs text-status-red">
                    {(s.param_errors ?? []).join(" ")}
                  </p>
                )}
                {(s.unresolved ?? []).length > 0 && (
                  <p className="text-xs text-text-muted">
                    {t("automations.plan.unresolved", {
                      refs: (s.unresolved ?? []).join(", "),
                    })}
                  </p>
                )}
                {s.params !== undefined && (
                  <details>
                    <summary className="cursor-pointer text-xs text-text-muted">
                      {t("automations.plan.params")}
                    </summary>
                    <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs">
                      {prettyJson(JSON.stringify(s.params))}
                    </pre>
                  </details>
                )}
              </li>
            );
          })}
        </ol>
      </div>
    </Dialog>
  );
};
