import React from "react";
import { useTranslation } from "react-i18next";
import { Settings2, Sparkles, X } from "lucide-react";
import type { Folder, RecipeItem, RecipeVar } from "@/bindings";
import { recipePlaceholders, splitTemplate } from "@/lib/meetingChat";

/** Titel mit Variablen als Beschriftung (`{{person}}` -> "Person"). */
export const recipeTitleText = (recipe: RecipeItem): string => {
  const vars = recipePlaceholders(recipe.spec);
  return splitTemplate(recipe.title)
    .map((part) =>
      part.kind === "text"
        ? part.text
        : (vars.find((v) => v.name === part.name)?.label ?? part.name),
    )
    .join("");
};

const RecipeTitle: React.FC<{ recipe: RecipeItem }> = ({ recipe }) => {
  const vars = recipePlaceholders(recipe.spec);
  return (
    <>
      {splitTemplate(recipe.title).map((part, i) =>
        part.kind === "text" ? (
          <React.Fragment key={i}>{part.text}</React.Fragment>
        ) : (
          <span key={i} className="italic text-text/60">
            {vars.find((v) => v.name === part.name)?.label ?? part.name}
          </span>
        ),
      )}
    </>
  );
};

// ---------------------------------------------------------------------------
// "/"-Menue
// ---------------------------------------------------------------------------

interface RecipeMenuProps {
  items: RecipeItem[];
  activeIndex: number;
  onPick: (recipe: RecipeItem) => void;
  onHover: (index: number) => void;
}

/** Auswahlliste unter dem Eingabefeld, solange die Eingabe mit `/` beginnt. */
export const RecipeMenu: React.FC<RecipeMenuProps> = ({
  items,
  activeIndex,
  onPick,
  onHover,
}) => {
  const { t } = useTranslation();
  return (
    <div
      role="listbox"
      aria-label={t("meetings.recipes.menu")}
      className="max-h-48 overflow-y-auto rounded-md border border-mid-gray/30 bg-background p-1 shadow-md"
    >
      {items.length === 0 ? (
        <p className="px-2 py-1 text-xs text-text/60">
          {t("meetings.recipes.noMatch")}
        </p>
      ) : (
        items.map((recipe, i) => (
          <div
            key={recipe.id}
            role="option"
            aria-selected={i === activeIndex}
            aria-label={recipeTitleText(recipe)}
            onMouseDown={(e) => {
              // Fokus bleibt im Eingabefeld.
              e.preventDefault();
              onPick(recipe);
            }}
            onMouseEnter={() => onHover(i)}
            className={`cursor-pointer rounded px-2 py-1 text-sm ${
              i === activeIndex ? "bg-logo-primary/20" : "hover:bg-mid-gray/15"
            }`}
          >
            <RecipeTitle recipe={recipe} />
          </div>
        ))
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Knopfleiste
// ---------------------------------------------------------------------------

interface RecipeBarProps {
  items: RecipeItem[];
  disabled: boolean;
  onPick: (recipe: RecipeItem) => void;
  onManage: () => void;
}

export const RecipeBar: React.FC<RecipeBarProps> = ({
  items,
  disabled,
  onPick,
  onManage,
}) => {
  const { t } = useTranslation();
  return (
    <div
      role="group"
      aria-label={t("meetings.recipes.bar")}
      className="flex flex-wrap items-center gap-1"
    >
      {items.map((recipe) => (
        <button
          key={recipe.id}
          type="button"
          disabled={disabled}
          onClick={() => onPick(recipe)}
          className="inline-flex max-w-full items-center gap-1 truncate rounded-full border border-mid-gray/40 px-2 py-0.5 text-xs text-text/70 hover:bg-mid-gray/15 hover:text-text disabled:opacity-50 cursor-pointer disabled:cursor-not-allowed"
        >
          <Sparkles width={10} height={10} aria-hidden="true" />
          <span className="truncate">{recipeTitleText(recipe)}</span>
        </button>
      ))}
      <button
        type="button"
        onClick={onManage}
        aria-label={t("meetings.recipes.manage")}
        title={t("meetings.recipes.manage")}
        className="rounded-full p-1 text-text/50 hover:bg-mid-gray/15 hover:text-text cursor-pointer"
      >
        <Settings2 width={12} height={12} aria-hidden="true" />
      </button>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Gewaehltes Recipe mit Variablen als Inline-Chips
// ---------------------------------------------------------------------------

interface RecipeChipProps {
  recipe: RecipeItem;
  values: Record<string, string>;
  folders: Folder[];
  onChange: (values: Record<string, string>) => void;
  onRemove: () => void;
  /** Enter in einem Variablenfeld sendet. */
  onSubmit: () => void;
}

const VariableInput: React.FC<{
  variable: RecipeVar;
  value: string;
  folders: Folder[];
  onChange: (value: string) => void;
  onSubmit: () => void;
}> = ({ variable, value, folders, onChange, onSubmit }) => {
  const { t } = useTranslation();
  const cls =
    "mx-0.5 rounded-full border border-logo-primary/60 bg-logo-primary/10 px-2 py-0 text-xs text-text focus:outline-none focus:ring-1 focus:ring-logo-primary";
  if (variable.kind === "folder") {
    // Ordner als waehlbare Chips (ein nativer <select> liesse sich nicht
    // gestalten); ein zweiter Klick hebt die Wahl auf.
    return (
      <span
        role="radiogroup"
        aria-label={variable.label}
        title={t("meetings.recipes.chooseFolder")}
        className="mx-0.5 inline-flex flex-wrap items-center gap-0.5"
      >
        {folders.map((f) => (
          <button
            key={f.id}
            type="button"
            role="radio"
            aria-checked={value === f.id}
            onClick={() => onChange(value === f.id ? "" : f.id)}
            className={`${cls} cursor-pointer ${
              value === f.id ? "bg-logo-primary/30" : "opacity-70"
            }`}
          >
            {f.name}
          </button>
        ))}
      </span>
    );
  }
  const isDate = variable.kind === "date_from" || variable.kind === "date_to";
  return (
    <input
      type={isDate ? "date" : "text"}
      aria-label={variable.label}
      placeholder={variable.label}
      value={value}
      size={isDate ? undefined : Math.max(8, value.length + 1)}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onSubmit();
        }
      }}
      className={cls}
    />
  );
};

/**
 * Das gewaehlte Recipe ueber dem Eingabefeld: der Titel, darin die Variablen
 * als ausfuellbare Chips; Variablen, die der Titel nicht nennt, folgen dahinter.
 */
export const RecipeChip: React.FC<RecipeChipProps> = ({
  recipe,
  values,
  folders,
  onChange,
  onRemove,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const vars = recipePlaceholders(recipe.spec);
  const titleParts = splitTemplate(recipe.title);
  const inTitle = new Set(
    titleParts.flatMap((p) => (p.kind === "var" ? [p.name] : [])),
  );
  const input = (variable: RecipeVar) => (
    <VariableInput
      key={variable.name}
      variable={variable}
      value={values[variable.name] ?? variable.default ?? ""}
      folders={folders}
      onChange={(value) => onChange({ ...values, [variable.name]: value })}
      onSubmit={onSubmit}
    />
  );
  const fallback = (name: string): RecipeVar => ({
    name,
    label: name,
    kind: "text",
    required: true,
  });

  return (
    <div
      data-testid="recipe-chip"
      className="flex flex-wrap items-center gap-y-1 rounded-md border border-logo-primary/40 bg-logo-primary/5 px-2 py-1 text-sm"
    >
      <Sparkles
        width={12}
        height={12}
        className="me-1 shrink-0 text-logo-primary"
        aria-hidden="true"
      />
      {titleParts.map((part, i) =>
        part.kind === "text" ? (
          <span key={i}>{part.text}</span>
        ) : (
          input(vars.find((v) => v.name === part.name) ?? fallback(part.name))
        ),
      )}
      {vars.filter((v) => !inTitle.has(v.name)).map(input)}
      <button
        type="button"
        onClick={onRemove}
        aria-label={t("meetings.recipes.remove")}
        title={t("meetings.recipes.remove")}
        className="ms-auto rounded-full p-0.5 text-text/50 hover:bg-mid-gray/20 hover:text-text cursor-pointer"
      >
        <X width={12} height={12} aria-hidden="true" />
      </button>
    </div>
  );
};
