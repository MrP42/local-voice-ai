import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Save, Trash2 } from "lucide-react";
import { type AutoTagOptions, type AutoTagPreset } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import {
  TAG_CATEGORIES,
  TAG_REGISTRY,
  localizedLabel,
} from "@/lib/tags/registry";

/** Voreinstellung: dokumentierte Tags, ausgewogen, kein Stil-Hinweis. */
export const defaultAutoTagOptions = (): AutoTagOptions => ({
  preferred_tags: TAG_REGISTRY.filter((t) => t.verified).map((t) => t.insert),
  coverage: "balanced",
  style_hint: "",
  max_per_sentence: 2,
});

interface AutoTagDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Aktuelle Einstellungen der Seite (persistiert in state.json). */
  options: AutoTagOptions;
  onOptionsChange: (next: AutoTagOptions) => void;
  /** Starten mit den gezeigten Einstellungen. */
  onRun: (options: AutoTagOptions) => void;
  uiLang: string;
}

const COVERAGES: AutoTagOptions["coverage"][] = ["sparse", "balanced", "rich"];

/**
 * Einstellungen des Auto-Taggings vor dem Lauf: Umfang, bevorzugte Tags,
 * Stil-Hinweis. Je Seite persistent; dazu Vorlagen (app-weit in den
 * Einstellungen), damit ein neues Projekt mit den Einstellungen des letzten
 * beginnen kann.
 */
export const AutoTagDialog: React.FC<AutoTagDialogProps> = ({
  open,
  onOpenChange,
  options,
  onOptionsChange,
  onRun,
  uiLang,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const presets = (getSetting("tts_autotag_presets") ?? []) as AutoTagPreset[];
  const [draft, setDraft] = useState<AutoTagOptions>(options);
  const [presetName, setPresetName] = useState("");
  const [selectedPreset, setSelectedPreset] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setDraft(options);
      setPresetName("");
    }
  }, [open, options]);

  const preferred = useMemo(() => new Set(draft.preferred_tags), [draft]);
  const toggleTag = (insert: string) => {
    const next = new Set(preferred);
    if (next.has(insert)) next.delete(insert);
    else next.add(insert);
    setDraft({ ...draft, preferred_tags: [...next] });
  };
  const setCategory = (category: string, on: boolean) => {
    const next = new Set(preferred);
    for (const tag of TAG_REGISTRY) {
      if (tag.category !== category) continue;
      if (on) next.add(tag.insert);
      else next.delete(tag.insert);
    }
    setDraft({ ...draft, preferred_tags: [...next] });
  };

  const savePreset = async () => {
    const name = presetName.trim();
    if (!name) return;
    const next = [
      ...presets.filter((p) => p.name !== name),
      { name, options: draft },
    ];
    await updateSetting("tts_autotag_presets", next);
    setSelectedPreset(name);
    setPresetName("");
    toast.success(t("tts.autotag.dialog.presetSaved", { name }));
  };
  const loadPreset = (name: string | null) => {
    setSelectedPreset(name);
    const preset = presets.find((p) => p.name === name);
    if (preset) setDraft(preset.options);
  };
  const deletePreset = async () => {
    if (!selectedPreset) return;
    await updateSetting(
      "tts_autotag_presets",
      presets.filter((p) => p.name !== selectedPreset),
    );
    setSelectedPreset(null);
  };

  const run = () => {
    onOptionsChange(draft);
    void updateSetting("tts_autotag_last", draft);
    onRun(draft);
    onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.autotag.dialog.title")}
      description={t("tts.autotag.dialog.description")}
      closeLabel={t("common.close")}
      className="max-w-2xl"
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("tts.stopConfirmCancel")}
          </Button>
          <Button
            onClick={run}
            disabled={draft.preferred_tags.length === 0}
            data-testid="autotag-run"
          >
            {t("tts.autotag.dialog.run")}
          </Button>
        </>
      }
    >
      <div className="space-y-4 text-sm">
        {/* Vorlagen */}
        <div className="space-y-1">
          <span className="font-medium">{t("tts.autotag.dialog.presets")}</span>
          <div className="flex flex-wrap items-center gap-2">
            <div className="w-56">
              <Select
                value={selectedPreset}
                options={presets.map((p) => ({ value: p.name, label: p.name }))}
                placeholder={t("tts.autotag.dialog.presetPick")}
                onChange={loadPreset}
                isClearable={true}
              />
            </div>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void deletePreset()}
              disabled={!selectedPreset}
              title={t("tts.autotag.dialog.presetDelete")}
            >
              <Trash2 width={14} height={14} />
            </Button>
            <Input
              type="text"
              value={presetName}
              onChange={(e) => setPresetName(e.target.value)}
              placeholder={t("tts.autotag.dialog.presetName")}
              className="w-44"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void savePreset()}
              disabled={!presetName.trim()}
            >
              <Save width={14} height={14} />
              {t("tts.autotag.dialog.presetSave")}
            </Button>
          </div>
        </div>

        {/* Umfang */}
        <div className="space-y-1">
          <span className="font-medium">
            {t("tts.autotag.dialog.coverage")}
          </span>
          <div className="flex flex-wrap gap-2">
            {COVERAGES.map((c) => (
              <label
                key={c}
                className={`flex cursor-pointer items-center gap-1.5 rounded-md border px-2 py-1 ${
                  draft.coverage === c
                    ? "border-logo-primary bg-logo-primary/10"
                    : "border-mid-gray/30"
                }`}
              >
                <input
                  type="radio"
                  name="autotag-coverage"
                  checked={draft.coverage === c}
                  onChange={() =>
                    setDraft({
                      ...draft,
                      coverage: c,
                      max_per_sentence:
                        c === "sparse" ? 1 : c === "rich" ? 3 : 2,
                    })
                  }
                />
                {t(`tts.autotag.dialog.coverage_${c}`)}
              </label>
            ))}
          </div>
        </div>

        {/* Stil */}
        <label className="block space-y-1">
          <span className="font-medium">{t("tts.autotag.dialog.style")}</span>
          <Input
            type="text"
            value={draft.style_hint}
            onChange={(e) => setDraft({ ...draft, style_hint: e.target.value })}
            placeholder={t("tts.autotag.dialog.stylePlaceholder")}
            className="w-full"
          />
        </label>

        {/* Bevorzugte Tags */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="font-medium">
              {t("tts.autotag.dialog.tags", {
                count: draft.preferred_tags.length,
              })}
            </span>
            <button
              type="button"
              className="text-xs text-logo-primary hover:underline"
              onClick={() =>
                setDraft({
                  ...draft,
                  preferred_tags: TAG_REGISTRY.filter((x) => x.verified).map(
                    (x) => x.insert,
                  ),
                })
              }
            >
              {t("tts.autotag.dialog.tagsVerifiedOnly")}
            </button>
          </div>
          <div className="max-h-64 space-y-2 overflow-y-auto rounded-md border border-mid-gray/20 p-2">
            {TAG_CATEGORIES.map((category) => {
              const tags = TAG_REGISTRY.filter(
                (x) => x.category === category.id,
              );
              const all = tags.every((x) => preferred.has(x.insert));
              return (
                <div key={category.id}>
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-medium text-text/70">
                      {t(`tts.tags.categories.${category.id}`)}
                    </span>
                    <button
                      type="button"
                      className="text-xs text-text/50 hover:text-text"
                      onClick={() => setCategory(category.id, !all)}
                    >
                      {all
                        ? t("tts.autotag.dialog.none")
                        : t("tts.autotag.dialog.all")}
                    </button>
                  </div>
                  <div className="mt-1 flex flex-wrap gap-1">
                    {tags.map((tag) => (
                      <button
                        key={tag.id}
                        type="button"
                        onClick={() => toggleTag(tag.insert)}
                        title={`[${tag.insert}]${tag.verified ? "" : ` · ${t("tts.autotag.dialog.unverified")}`}`}
                        className={`rounded px-1.5 py-0.5 text-xs ${
                          preferred.has(tag.insert)
                            ? "bg-logo-primary/20 text-text"
                            : "bg-mid-gray/10 text-text/50"
                        } ${tag.verified ? "" : "italic"}`}
                      >
                        {localizedLabel(tag, uiLang)}
                      </button>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </Dialog>
  );
};
