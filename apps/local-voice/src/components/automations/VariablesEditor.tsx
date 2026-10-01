import React, { useId } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import {
  issuesUnder,
  listToText,
  textToInt,
  textToList,
  type Issue,
  type VariableDecl,
} from "./model";

interface VariablesEditorProps {
  variables: Record<string, VariableDecl>;
  issues: Issue[];
  onChange: (variables: Record<string, VariableDecl> | undefined) => void;
}

const TYPES: VariableDecl["type"][] = ["string", "number", "bool", "list"];

/** Neue Kennung `variable`, `variable_2`, ... */
const freeName = (existing: string[]): string => {
  if (!existing.includes("variable")) return "variable";
  for (let n = 2; n < 100; n++) {
    if (!existing.includes(`variable_${n}`)) return `variable_${n}`;
  }
  return `variable_${existing.length + 1}`;
};

const DefaultInput: React.FC<{
  decl: VariableDecl;
  label: string;
  testId: string;
  onChange: (value: unknown) => void;
}> = ({ decl, label, testId, onChange }) => {
  if (decl.type === "bool") {
    return (
      <input
        type="checkbox"
        aria-label={label}
        checked={decl.default === true}
        onChange={(e) => onChange(e.target.checked)}
        data-testid={testId}
      />
    );
  }
  if (decl.type === "number") {
    return (
      <Input
        type="number"
        aria-label={label}
        value={typeof decl.default === "number" ? String(decl.default) : ""}
        onChange={(e) => onChange(textToInt(e.target.value))}
        data-testid={testId}
      />
    );
  }
  if (decl.type === "list") {
    return (
      <Input
        aria-label={label}
        className="w-full"
        defaultValue={listToText(decl.default)}
        onBlur={(e) => onChange(textToList(e.target.value))}
        data-testid={testId}
      />
    );
  }
  return (
    <Input
      aria-label={label}
      className="w-full"
      value={typeof decl.default === "string" ? decl.default : ""}
      onChange={(e) => onChange(e.target.value)}
      data-testid={testId}
    />
  );
};

/** Variablen des Ablaufs: Name, Art, Vorgabe, Beschreibung. Sie stehen in Vorlagen als
 *  `{{vars.<name>}}` und werden bei jedem Start (auch von Hand oder durch einen Agenten)
 *  mitgegeben, wenn sie keine Vorgabe haben. */
export const VariablesEditor: React.FC<VariablesEditorProps> = ({
  variables,
  issues,
  onChange,
}) => {
  const { t } = useTranslation();
  const uid = useId();
  const names = Object.keys(variables);

  const commit = (next: Record<string, VariableDecl>) =>
    onChange(Object.keys(next).length > 0 ? next : undefined);

  const rename = (from: string, to: string) => {
    const next: Record<string, VariableDecl> = {};
    for (const [k, v] of Object.entries(variables)) {
      next[k === from ? to : k] = v;
    }
    commit(next);
  };

  const patch = (name: string, change: Partial<VariableDecl>) => {
    const decl: VariableDecl = { ...variables[name], ...change };
    for (const k of Object.keys(decl) as (keyof VariableDecl)[]) {
      if (decl[k] === undefined || decl[k] === "") delete decl[k];
    }
    commit({ ...variables, [name]: decl });
  };

  return (
    <div className="space-y-3">
      {names.length === 0 && (
        <p className="text-sm text-text-muted" data-testid="variables-empty">
          {t("automations.variables.empty")}
        </p>
      )}
      <ul className="space-y-3">
        {names.map((name, index) => {
          const decl = variables[name];
          const mine = issuesUnder(issues, `/variables/${name}`);
          return (
            <li
              key={`${index}`}
              className="grid gap-2 rounded-lg border border-mid-gray/30 p-3 sm:grid-cols-[1fr_9rem_1fr_auto]"
              data-testid="variable-row"
            >
              <div className="min-w-0 space-y-1">
                <label htmlFor={`${uid}-n${index}`} className="block text-xs">
                  {t("automations.variables.name")}
                </label>
                <Input
                  id={`${uid}-n${index}`}
                  className="w-full"
                  value={name}
                  aria-invalid={mine.length > 0 || undefined}
                  onChange={(e) => rename(name, e.target.value)}
                  data-testid={`variable-${index}-name`}
                />
              </div>
              <div className="min-w-0 space-y-1">
                <span className="block text-xs">
                  {t("automations.variables.type")}
                </span>
                <Select
                  value={decl.type}
                  options={TYPES.map((v) => ({
                    value: v,
                    label: t(`automations.variables.types.${v}`),
                  }))}
                  ariaLabel={t("automations.variables.type")}
                  onChange={(v) =>
                    v &&
                    patch(name, {
                      type: v as VariableDecl["type"],
                      default: undefined,
                    })
                  }
                  menuPortal
                />
              </div>
              <div className="min-w-0 space-y-1">
                <span className="block text-xs">
                  {t("automations.variables.default")}
                </span>
                <DefaultInput
                  decl={decl}
                  label={t("automations.variables.default")}
                  testId={`variable-${index}-default`}
                  onChange={(value) => patch(name, { default: value })}
                />
              </div>
              <div className="flex items-end">
                <Button
                  variant="danger-ghost"
                  size="sm"
                  aria-label={t("automations.variables.remove", { name })}
                  onClick={() => {
                    const next = { ...variables };
                    delete next[name];
                    commit(next);
                  }}
                  data-testid={`variable-${index}-remove`}
                >
                  <Trash2 size={14} aria-hidden="true" />
                </Button>
              </div>
              <div className="min-w-0 space-y-1 sm:col-span-4">
                <label htmlFor={`${uid}-d${index}`} className="block text-xs">
                  {t("automations.variables.description")}
                </label>
                <Input
                  id={`${uid}-d${index}`}
                  className="w-full"
                  value={decl.description ?? ""}
                  onChange={(e) => patch(name, { description: e.target.value })}
                />
                {mine.length > 0 && (
                  <p className="text-xs text-status-red">
                    {mine.map((i) => i.message).join(" ")}
                  </p>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      <Button
        variant="secondary"
        size="sm"
        onClick={() =>
          commit({ ...variables, [freeName(names)]: { type: "string" } })
        }
        data-testid="variable-add"
      >
        <Plus size={14} aria-hidden="true" />
        {t("automations.variables.add")}
      </Button>
    </div>
  );
};
