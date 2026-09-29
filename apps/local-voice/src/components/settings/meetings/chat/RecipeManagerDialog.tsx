import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";
import {
  commands,
  type RecipeItem,
  type RecipeScope,
  type RecipeVar,
  type RecipeVarKind,
} from "@/bindings";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Textarea } from "../../../ui/Textarea";
import Badge from "../../../ui/Badge";
import { Select } from "../../../ui/Select";
import { recipeTitleText } from "./RecipeMenu";

const SCOPES: RecipeScope[] = ["meeting", "global", "any"];
const KINDS: RecipeVarKind[] = [
  "text",
  "person",
  "folder",
  "date_from",
  "date_to",
  "meeting",
];

interface Draft {
  id: string | null;
  title: string;
  prompt: string;
  scope: RecipeScope;
  liveOk: boolean;
  variables: RecipeVar[];
}

const draftOf = (recipe: RecipeItem | null): Draft => ({
  id: recipe?.id ?? null,
  title: recipe?.title ?? "",
  prompt: recipe?.spec.prompt ?? "",
  scope: recipe?.spec.scope ?? "any",
  liveOk: recipe?.spec.live_ok ?? false,
  variables: (recipe?.spec.variables ?? []).map((v) => ({
    name: v.name,
    label: v.label,
    kind: v.kind,
    required: v.required ?? true,
    default: v.default ?? null,
  })),
});

/** Fehler von `chat_recipes_*` als Text (`recipe_invalid:<grund>` usw.). */
const recipeErrorText = (
  error: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string => {
  const [code, ...rest] = String(error).split(":");
  const reason = rest.join(":").trim();
  switch (code.trim()) {
    case "recipe_invalid":
      return t("meetings.recipes.errors.invalid", { reason });
    case "recipe_readonly":
      return t("meetings.recipes.errors.readonly");
    case "recipe_not_found":
      return t("meetings.recipes.errors.not_found");
    default:
      return t("meetings.recipes.errors.generic", { error });
  }
};

interface RecipeManagerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Nach jeder Aenderung (Liste im Chat neu laden). */
  onChanged: () => void;
}

/**
 * Recipes verwalten: Liste, Duplizieren, Bearbeiten, Loeschen. Mitgelieferte
 * sind schreibgeschuetzt und lassen sich nur duplizieren (wie Vorlagen).
 */
export const RecipeManagerDialog: React.FC<RecipeManagerDialogProps> = ({
  open,
  onOpenChange,
  onChanged,
}) => {
  const { t } = useTranslation();
  const tt = useCallback(
    (key: string, options?: Record<string, unknown>) => t(key, options),
    [t],
  );
  const [items, setItems] = useState<RecipeItem[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const result = await commands.chatRecipesList();
    if (result.status === "ok") setItems(result.data ?? []);
  }, []);

  useEffect(() => {
    if (!open) return;
    setDraft(null);
    setError(null);
    void load();
  }, [open, load]);

  const run = async (
    action: () => Promise<{ status: string; error?: string }>,
  ) => {
    setBusy(true);
    setError(null);
    const result = await action();
    setBusy(false);
    if (result.status === "error") {
      setError(recipeErrorText(result.error ?? "", tt));
      return false;
    }
    await load();
    onChanged();
    return true;
  };

  const duplicate = (id: string) =>
    void run(() => commands.chatRecipesDuplicate(id));

  const remove = (id: string) => void run(() => commands.chatRecipesDelete(id));

  const save = async () => {
    if (!draft) return;
    const ok = await run(() =>
      commands.chatRecipesSave(draft.id, draft.title.trim(), {
        version: 1,
        prompt: draft.prompt,
        variables: draft.variables.map((v) => ({
          ...v,
          name: v.name.trim(),
          label: v.label.trim() || v.name.trim(),
        })),
        scope: draft.scope,
        live_ok: draft.liveOk,
      }),
    );
    if (ok) setDraft(null);
  };

  const patchVar = (index: number, patch: Partial<RecipeVar>) =>
    setDraft((d) =>
      d
        ? {
            ...d,
            variables: d.variables.map((v, i) =>
              i === index ? { ...v, ...patch } : v,
            ),
          }
        : d,
    );

  const labelCls = "text-xs font-medium text-text/70";

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.recipes.manage")}
      closeLabel={t("meetings.recipes.close")}
      contentClassName="max-w-2xl"
      footer={
        draft ? (
          <>
            <Button variant="secondary" onClick={() => setDraft(null)}>
              {t("meetings.recipes.cancel")}
            </Button>
            <Button
              onClick={() => void save()}
              disabled={
                busy || draft.title.trim() === "" || draft.prompt.trim() === ""
              }
            >
              {t("meetings.recipes.save")}
            </Button>
          </>
        ) : (
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("meetings.recipes.close")}
          </Button>
        )
      }
    >
      <div className="space-y-3">
        {error && (
          <p role="alert" className="text-sm text-red-400">
            {error}
          </p>
        )}
        {draft ? (
          <div className="space-y-3">
            <label className="block space-y-1">
              <span className={labelCls}>
                {t("meetings.recipes.fieldTitle")}
              </span>
              <Input
                aria-label={t("meetings.recipes.fieldTitle")}
                value={draft.title}
                onChange={(e) => setDraft({ ...draft, title: e.target.value })}
                className="w-full"
              />
            </label>
            <label className="block space-y-1">
              <span className={labelCls}>
                {t("meetings.recipes.fieldPrompt")}
              </span>
              <Textarea
                aria-label={t("meetings.recipes.fieldPrompt")}
                value={draft.prompt}
                onChange={(e) => setDraft({ ...draft, prompt: e.target.value })}
                className="w-full"
                variant="compact"
              />
              <span className="block text-xs text-text/50">
                {t("meetings.recipes.promptHint", { example: "{{person}}" })}
              </span>
            </label>
            <div className="flex flex-wrap items-center gap-4">
              <div className="flex items-center gap-2">
                <span className={labelCls}>
                  {t("meetings.recipes.fieldScope")}
                </span>
                <div
                  role="group"
                  aria-label={t("meetings.recipes.fieldScope")}
                  className="inline-flex rounded-lg border border-mid-gray/20 p-0.5 text-xs"
                >
                  {SCOPES.map((scope) => (
                    <button
                      key={scope}
                      type="button"
                      aria-pressed={draft.scope === scope}
                      onClick={() => setDraft({ ...draft, scope })}
                      className={`rounded-md px-2 py-0.5 cursor-pointer ${
                        draft.scope === scope
                          ? "bg-logo-primary/20 text-text"
                          : "text-text/60 hover:text-text"
                      }`}
                    >
                      {t(`meetings.recipes.scope.${scope}`)}
                    </button>
                  ))}
                </div>
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={draft.liveOk}
                  onChange={(e) =>
                    setDraft({ ...draft, liveOk: e.target.checked })
                  }
                />
                {t("meetings.recipes.liveOk")}
              </label>
            </div>
            <div className="space-y-2">
              <p className={labelCls}>{t("meetings.recipes.variables")}</p>
              {draft.variables.map((v, i) => (
                <div
                  key={i}
                  className="flex flex-wrap items-center gap-2"
                  data-testid="recipe-variable"
                >
                  <Input
                    variant="compact"
                    aria-label={t("meetings.recipes.varName")}
                    placeholder={t("meetings.recipes.varName")}
                    value={v.name}
                    onChange={(e) => patchVar(i, { name: e.target.value })}
                    className="w-32"
                  />
                  <Input
                    variant="compact"
                    aria-label={t("meetings.recipes.varLabel")}
                    placeholder={t("meetings.recipes.varLabel")}
                    value={v.label}
                    onChange={(e) => patchVar(i, { label: e.target.value })}
                    className="w-36"
                  />
                  <div className="w-40" title={t("meetings.recipes.varKind")}>
                    <Select
                      value={v.kind}
                      options={KINDS.map((k) => ({
                        value: k,
                        label: t(`meetings.recipes.kind.${k}`),
                      }))}
                      onChange={(value) =>
                        patchVar(i, {
                          kind: (value ?? "text") as RecipeVarKind,
                        })
                      }
                    />
                  </div>
                  <label className="flex items-center gap-1 text-xs">
                    <input
                      type="checkbox"
                      checked={v.required ?? true}
                      onChange={(e) =>
                        patchVar(i, { required: e.target.checked })
                      }
                    />
                    {t("meetings.recipes.varRequired")}
                  </label>
                  <button
                    type="button"
                    aria-label={t("meetings.recipes.removeVariable")}
                    title={t("meetings.recipes.removeVariable")}
                    onClick={() =>
                      setDraft({
                        ...draft,
                        variables: draft.variables.filter((_, j) => j !== i),
                      })
                    }
                    className="rounded-md p-1 text-text/50 hover:text-red-400 cursor-pointer"
                  >
                    <Trash2 width={14} height={14} />
                  </button>
                </div>
              ))}
              <Button
                size="sm"
                variant="secondary"
                onClick={() =>
                  setDraft({
                    ...draft,
                    variables: [
                      ...draft.variables,
                      {
                        name: "",
                        label: "",
                        kind: "text",
                        required: true,
                        default: null,
                      },
                    ],
                  })
                }
              >
                <Plus width={12} height={12} />
                {t("meetings.recipes.addVariable")}
              </Button>
            </div>
          </div>
        ) : (
          <>
            <div className="flex items-center justify-between gap-2">
              <p className="text-xs text-text/60">
                {t("meetings.recipes.readonly")}
              </p>
              <Button size="sm" onClick={() => setDraft(draftOf(null))}>
                <Plus width={12} height={12} />
                {t("meetings.recipes.new")}
              </Button>
            </div>
            {items.length === 0 ? (
              <p className="text-sm text-text/60">
                {t("meetings.recipes.empty")}
              </p>
            ) : (
              <ul className="divide-y divide-mid-gray/20">
                {items.map((recipe) => (
                  <li
                    key={recipe.id}
                    data-recipe-id={recipe.id}
                    className="flex items-center justify-between gap-2 py-1.5"
                  >
                    <span className="min-w-0 truncate text-sm">
                      {recipeTitleText(recipe)}
                      {recipe.builtin && (
                        <Badge variant="secondary" className="ms-2">
                          {t("meetings.recipes.builtin")}
                        </Badge>
                      )}
                    </span>
                    <span className="flex shrink-0 items-center gap-1">
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busy}
                        onClick={() => duplicate(recipe.id)}
                      >
                        {t("meetings.recipes.duplicate")}
                      </Button>
                      {!recipe.builtin && (
                        <>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => setDraft(draftOf(recipe))}
                          >
                            {t("meetings.recipes.edit")}
                          </Button>
                          <Button
                            size="sm"
                            variant="danger-ghost"
                            disabled={busy}
                            onClick={() => remove(recipe.id)}
                          >
                            {t("meetings.recipes.delete")}
                          </Button>
                        </>
                      )}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
};
