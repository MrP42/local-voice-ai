import React, { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { IntegrationView, WorkflowFieldSpec } from "@/bindings";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import { Textarea } from "../ui/Textarea";
import {
  anyToText,
  isTemplate,
  listToText,
  textToAny,
  textToInt,
  textToList,
} from "./model";

interface FieldInputProps {
  spec: WorkflowFieldSpec;
  value: unknown;
  /** `undefined` entfernt das Feld. */
  onChange: (value: unknown) => void;
  /** Meldungen der Pruefung zu genau diesem Feld. */
  issues: string[];
  integrations: IntegrationView[];
  /** Praefix fuer `data-testid` (`trigger` oder `step-<i>`). */
  testPrefix: string;
}

const LONG_TEXT = new Set(["body", "text", "content"]);

/**
 * Ein Feld des Katalogs als Formularfeld. Die Art des Felds (Text, Zahl, Ja/Nein, Auswahl,
 * Integration, Liste, frei) kommt aus dem Katalog des Backends: ein neuer Baustein braucht
 * keine Aenderung hier. Nicht-feste Felder nehmen statt des Werts auch einen Text mit
 * `{{...}}` an (wird erst beim Lauf eingesetzt); dafuer gibt es den Umschalter „Variable“.
 */
export const FieldInput: React.FC<FieldInputProps> = ({
  spec,
  value,
  onChange,
  issues,
  integrations,
  testPrefix,
}) => {
  const { t } = useTranslation();
  const id = useId();
  const label = t(`automations.fields.${spec.name}.label`, {
    defaultValue: spec.name,
  });
  const hint = t(`automations.fields.${spec.name}.hint`, { defaultValue: "" });
  const invalid = issues.length > 0;
  const typed = ["id", "int", "bool", "choice"].includes(spec.kind);
  const canDynamic = typed && !spec.literal;
  const [dynamic, setDynamic] = useState(isTemplate(value));
  // Eine von aussen gesetzte Vorlage (Import, Vorlagenwahl) schaltet auf Textfeld.
  useEffect(() => {
    if (isTemplate(value)) setDynamic(true);
  }, [value]);

  const describedBy = [hint ? `${id}-hint` : null, invalid ? `${id}-err` : null]
    .filter(Boolean)
    .join(" ");
  const common = {
    id,
    "aria-invalid": invalid || undefined,
    "aria-describedby": describedBy || undefined,
    "data-testid": `${testPrefix}-${spec.name}`,
  };

  let control: React.ReactNode;
  if (canDynamic && dynamic) {
    control = (
      <Input
        {...common}
        className="w-full"
        value={typeof value === "string" ? value : ""}
        placeholder="{{trigger.…}}"
        onChange={(e) => onChange(e.target.value)}
      />
    );
  } else if (spec.kind === "bool") {
    control = (
      <label className="flex items-center gap-2 text-sm">
        <input
          {...common}
          type="checkbox"
          checked={value === true}
          onChange={(e) => onChange(e.target.checked ? true : undefined)}
        />
        <span className="text-text-muted">{t("automations.editor.yes")}</span>
      </label>
    );
  } else if (spec.kind === "choice") {
    control = (
      <Select
        value={typeof value === "string" ? value : null}
        options={spec.options.map((o) => ({
          value: o,
          label: t(`automations.choices.${spec.name}.${o}`, {
            defaultValue: o,
          }),
        }))}
        isClearable={!spec.required}
        placeholder={t("automations.editor.choose")}
        ariaLabel={label}
        onChange={(v) => onChange(v ?? undefined)}
        menuPortal
      />
    );
  } else if (spec.kind === "id") {
    const usable = integrations.filter(
      (v) =>
        !spec.capability ||
        v.capabilities.some((c) => c.capability === spec.capability),
    );
    const options = usable.map((v) => ({
      value: v.integration.id,
      label: `${v.integration.label} (${v.integration.id})`,
    }));
    const current = typeof value === "string" ? value : null;
    if (current && !options.some((o) => o.value === current)) {
      options.unshift({
        value: current,
        label: t("automations.editor.unknownIntegration", { id: current }),
      });
    }
    control = (
      <Select
        value={current}
        options={options}
        isClearable={!spec.required}
        placeholder={t("automations.editor.chooseIntegration")}
        ariaLabel={label}
        onChange={(v) => onChange(v ?? undefined)}
        menuPortal
      />
    );
  } else if (spec.kind === "int") {
    control = (
      <Input
        {...common}
        type="number"
        inputMode="numeric"
        min={spec.min ?? undefined}
        max={spec.max ?? undefined}
        value={typeof value === "number" ? String(value) : ""}
        onChange={(e) => onChange(textToInt(e.target.value))}
        className="w-32"
      />
    );
  } else if (spec.kind === "text_list") {
    control = (
      <CommitText
        {...common}
        initial={listToText(value)}
        placeholder={t("automations.editor.listPlaceholder")}
        onCommit={(text) => onChange(textToList(text))}
      />
    );
  } else if (spec.kind === "any") {
    control = (
      <CommitText
        {...common}
        initial={anyToText(value)}
        placeholder={t("automations.editor.anyPlaceholder")}
        onCommit={(text) => onChange(textToAny(text))}
      />
    );
  } else if (LONG_TEXT.has(spec.name)) {
    control = (
      <Textarea
        {...common}
        className="w-full"
        variant="compact"
        value={typeof value === "string" ? value : ""}
        onChange={(e) => onChange(e.target.value)}
      />
    );
  } else {
    control = (
      <Input
        {...common}
        className="w-full"
        value={typeof value === "string" ? value : ""}
        onChange={(e) => onChange(e.target.value)}
      />
    );
  }

  return (
    <div className="min-w-0 space-y-1" data-field={spec.name}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <label htmlFor={id} className="block text-sm font-medium">
          {label}
          {spec.required && (
            <span aria-hidden="true" className="text-status-red">
              {" *"}
            </span>
          )}
        </label>
        {canDynamic && (
          <button
            type="button"
            className="text-xs text-text-muted underline decoration-dotted hover:text-text"
            aria-pressed={dynamic}
            onClick={() => {
              setDynamic(!dynamic);
              onChange(undefined);
            }}
            data-testid={`${testPrefix}-${spec.name}-dynamic`}
          >
            {dynamic
              ? t("automations.editor.fixedValue")
              : t("automations.editor.fromVariable")}
          </button>
        )}
      </div>
      {control}
      {hint && (
        <p id={`${id}-hint`} className="text-xs text-text-muted">
          {hint}
        </p>
      )}
      {invalid && (
        <p
          id={`${id}-err`}
          className="text-xs text-status-red"
          data-testid={`${testPrefix}-${spec.name}-error`}
        >
          {issues.join(" ")}
        </p>
      )}
    </div>
  );
};

/** Textfeld, das erst beim Verlassen (oder mit Enter) uebernommen wird: Kommas und Klammern
 *  duerfen beim Tippen kurz „unfertig“ sein. */
const CommitText: React.FC<{
  id: string;
  initial: string;
  placeholder?: string;
  onCommit: (text: string) => void;
  "aria-invalid"?: boolean;
  "aria-describedby"?: string;
  "data-testid"?: string;
}> = ({ initial, onCommit, ...rest }) => {
  const [text, setText] = useState(initial);
  useEffect(() => setText(initial), [initial]);
  return (
    <Input
      {...rest}
      className="w-full"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => onCommit(text)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onCommit(text);
      }}
    />
  );
};
