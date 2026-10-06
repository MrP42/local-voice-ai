import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  AudioLines,
  ChevronDown,
  Download,
  Globe,
  Languages,
  RefreshCw,
  Trash2,
} from "lucide-react";
import type { ModelInfo } from "@/bindings";
import type { ModelCardStatus } from "@/components/onboarding";
import { useModelStore } from "@/stores/modelStore";
import { formatModelSize } from "@/lib/utils/format";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import {
  getLanguageLabel,
  getUniqueCapabilityLanguages,
  MODEL_CAPABILITY_LANGUAGES,
  supportsLanguageCode,
} from "@/lib/constants/languages.ts";
import { Button } from "../../ui/Button";
import type { ActionMenuItem } from "../../ui/ActionMenu";
import { ModelRow, RowChip, type RowProgress } from "./ModelRow";
import { ModelSection } from "./ModelSection";
import { ModelOptions } from "./ModelOptions";
import { ActiveBar } from "./ActiveBar";

// Legacy = ein Blob-Download (.bin/ONNX), vom Katalog-GGUF abgeloest: nur noch
// sichtbar, wenn er schon auf der Platte liegt.
const isLegacy = (model: ModelInfo): boolean =>
  typeof model.source === "object" && "Url" in model.source;

const languageSummary = (
  model: ModelInfo,
  t: (k: string, o?: Record<string, unknown>) => string,
) => {
  const langs = getUniqueCapabilityLanguages(model.supported_languages);
  if (langs.length === 0) return null;
  if (langs.length === 1) return getLanguageLabel(langs[0]) || langs[0];
  return t("modelSelector.capabilities.languageCount", { total: langs.length });
};

/**
 * Diktatmodelle: aktives Modell oben, Installiertes als Zeilen, Ladbares
 * zugeklappt. Sprachfilter und "Neu einlesen" bleiben im Kopf der Gruppe.
 */
export const DictationSection: React.FC<{ query: string }> = ({ query }) => {
  const { t } = useTranslation();
  const {
    models,
    currentModel,
    downloadingModels,
    downloadProgress,
    downloadStats,
    verifyingModels,
    extractingModels,
    isRescanning,
    downloadModel,
    cancelDownload,
    selectModel,
    deleteModel,
    rescanLocalModels,
  } = useModelStore();
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [languageFilter, setLanguageFilter] = useState("all");

  const status = (id: string): ModelCardStatus => {
    if (id in extractingModels) return "extracting";
    if (id in verifyingModels) return "verifying";
    if (id in downloadingModels) return "downloading";
    if (switchingId === id) return "switching";
    if (id === currentModel) return "active";
    return models.find((m) => m.id === id)?.is_downloaded
      ? "available"
      : "downloadable";
  };

  const select = async (id: string) => {
    setSwitchingId(id);
    try {
      await selectModel(id);
    } finally {
      setSwitchingId(null);
    }
  };

  const remove = async (model: ModelInfo) => {
    const modelName = model.name || model.id;
    const ok = await ask(
      model.id === currentModel
        ? t("settings.models.deleteActiveConfirm", { modelName })
        : t("settings.models.deleteConfirm", { modelName }),
      { title: t("settings.models.deleteTitle"), kind: "warning" },
    );
    if (ok) await deleteModel(model.id).catch(() => {});
  };

  const q = query.trim().toLowerCase();
  const { installed, available } = useMemo(() => {
    const installed: ModelInfo[] = [];
    const available: ModelInfo[] = [];
    for (const m of models) {
      if (isLegacy(m) && !m.is_downloaded) continue;
      if (
        languageFilter !== "all" &&
        !supportsLanguageCode(m.supported_languages, languageFilter)
      )
        continue;
      // Die id traegt den Namen des Herstellers ("nemotron-3.5-asr") -- auch
      // danach soll man suchen koennen.
      if (q && !`${m.name} ${m.description} ${m.id}`.toLowerCase().includes(q))
        continue;
      if (
        m.is_custom ||
        m.is_downloaded ||
        m.id in downloadingModels ||
        m.id in extractingModels
      )
        installed.push(m);
      else available.push(m);
    }
    installed.sort((a, b) =>
      a.id === currentModel
        ? -1
        : b.id === currentModel
          ? 1
          : Number(a.is_custom) - Number(b.is_custom),
    );
    available.sort(
      (a, b) => Number(b.is_recommended) - Number(a.is_recommended),
    );
    return { installed, available };
  }, [
    models,
    languageFilter,
    q,
    downloadingModels,
    extractingModels,
    currentModel,
  ]);

  const active = models.find((m) => m.id === currentModel) ?? null;

  const row = (model: ModelInfo) => {
    const st = status(model.id);
    const name = getTranslatedModelName(model, t);
    const menu: ActionMenuItem[] = [];
    if (st === "available" || st === "active") {
      menu.push({
        id: "delete",
        label: t("common.delete"),
        icon: Trash2,
        onSelect: () => void remove(model),
      });
    }
    const progress: RowProgress | null =
      st === "downloading"
        ? {
            kind: "download",
            percent: downloadProgress[model.id]?.percentage ?? 0,
            speed: downloadStats[model.id]?.speed,
            onCancel: () => void cancelDownload(model.id).catch(() => {}),
          }
        : st === "verifying"
          ? { kind: "busy", label: t("modelSelector.verifyingGeneric") }
          : st === "extracting"
            ? { kind: "busy", label: t("modelSelector.extractingGeneric") }
            : st === "switching"
              ? { kind: "busy", label: t("modelSelector.switching") }
              : null;
    const langs = languageSummary(model, t);
    return (
      <ModelRow
        key={model.id}
        name={name}
        active={st === "active"}
        dataAttrs={{ "data-asr-model": model.id, "data-status": st }}
        badges={
          <>
            {st === "active" && (
              <RowChip tone="accent">{t("modelSelector.active")}</RowChip>
            )}
            {st === "downloadable" && model.is_recommended && (
              <RowChip tone="accent">{t("onboarding.recommended")}</RowChip>
            )}
            {model.is_custom && <RowChip>{t("modelSelector.custom")}</RowChip>}
            {isLegacy(model) && <RowChip>{t("modelSelector.legacy")}</RowChip>}
            {model.license_non_commercial && (
              <RowChip tone="warning">
                {t("modelSelector.nonCommercial")}
              </RowChip>
            )}
          </>
        }
        meta={
          <>
            {langs && (
              <span className="flex items-center gap-1">
                <Globe className="h-3.5 w-3.5" />
                {langs}
              </span>
            )}
            <span className="tabular-nums">
              {formatModelSize(Number(model.size_mb))}
            </span>
          </>
        }
        primary={
          st === "available" ? (
            <Button
              variant="primary"
              size="sm"
              onClick={() => void select(model.id)}
            >
              {t("settings.models.row.use")}
            </Button>
          ) : st === "downloadable" ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void downloadModel(model.id)}
              className="flex items-center gap-1.5"
            >
              <Download className="h-3.5 w-3.5" />
              {t("settings.models.row.download")}
            </Button>
          ) : null
        }
        menu={menu}
        progress={progress}
        details={
          <>
            <p>{getTranslatedModelDescription(model, t)}</p>
            {model.license_non_commercial && (
              <p
                className="text-amber-700 dark:text-amber-400"
                data-testid="model-nc-note"
              >
                {t("modelSelector.nonCommercialHint", {
                  license: (model.license ?? "").toUpperCase(),
                })}
              </p>
            )}
            <p className="flex flex-wrap items-center gap-3 text-xs text-text/50">
              {model.supports_translation && (
                <span className="flex items-center gap-1">
                  <Languages className="h-3.5 w-3.5" />
                  {t("modelSelector.capabilities.translate")}
                </span>
              )}
              {model.supports_streaming && (
                <span className="flex items-center gap-1">
                  <AudioLines className="h-3.5 w-3.5" />
                  {t("modelSelector.streaming")}
                </span>
              )}
              {model.accuracy_score > 0 && (
                <span>
                  {t("onboarding.modelCard.accuracy")}{" "}
                  {Math.round(model.accuracy_score * 100)} %
                </span>
              )}
              {model.speed_score > 0 && (
                <span>
                  {t("onboarding.modelCard.speed")}{" "}
                  {Math.round(model.speed_score * 100)} %
                </span>
              )}
            </p>
            {/* Nur das Modell in Betrieb: diese Einstellungen gelten global. */}
            {st === "active" && <ModelOptions model={model} />}
          </>
        }
      />
    );
  };

  return (
    <div className="space-y-5" data-models-section="dictation">
      <ActiveBar
        label={t("settings.models.active.dictation")}
        name={active ? getTranslatedModelName(active, t) : null}
      />
      <ModelSection
        id="asr-installed"
        title={t("settings.models.groups.installed")}
        count={installed.length}
        collapsible={false}
        actions={
          <>
            <button
              type="button"
              onClick={() => rescanLocalModels()}
              disabled={isRescanning}
              title={t("settings.models.rescan.tooltip")}
              className="flex items-center gap-1.5 rounded-lg bg-mid-gray/10 px-2.5 py-1 text-xs font-medium text-text/60 hover:bg-mid-gray/20 disabled:opacity-50"
            >
              <RefreshCw
                className={`h-3.5 w-3.5 ${isRescanning ? "animate-spin" : ""}`}
              />
              {t("settings.models.rescan.label")}
            </button>
            <LanguageFilter
              value={languageFilter}
              onChange={setLanguageFilter}
            />
          </>
        }
      >
        {installed.length > 0 ? (
          installed.map(row)
        ) : (
          <p className="px-3 py-3 text-sm text-text/50">
            {t("settings.models.noModelsMatch")}
          </p>
        )}
      </ModelSection>
      {available.length > 0 && (
        <ModelSection
          id="asr-available"
          title={t("settings.models.groups.available")}
          count={available.length}
          defaultOpen={false}
          forceOpen={q !== ""}
        >
          {available.map(row)}
        </ModelSection>
      )}
    </div>
  );
};

/** Sprachfilter der Diktatmodelle (durchsuchbar). */
const LanguageFilter: React.FC<{
  value: string;
  onChange: (v: string) => void;
}> = ({ value, onChange }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) {
        setOpen(false);
        setSearch("");
      }
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  const langs = MODEL_CAPABILITY_LANGUAGES.filter((l) =>
    l.label.toLowerCase().includes(search.toLowerCase()),
  );
  const pick = (v: string) => {
    onChange(v);
    setOpen(false);
    setSearch("");
  };
  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium ${
          value !== "all"
            ? "bg-logo-primary/20 text-logo-primary"
            : "bg-mid-gray/10 text-text/60 hover:bg-mid-gray/20"
        }`}
      >
        <Globe className="h-3.5 w-3.5" />
        <span className="max-w-[120px] truncate">
          {value === "all"
            ? t("settings.models.filters.allLanguages")
            : getLanguageLabel(value)}
        </span>
        <ChevronDown className={`h-3.5 w-3.5 ${open ? "rotate-180" : ""}`} />
      </button>
      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 w-56 overflow-hidden rounded-lg border border-mid-gray/80 bg-background shadow-lg">
          <div className="border-b border-mid-gray/40 p-2">
            <input
              ref={inputRef}
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && langs.length > 0) pick(langs[0].value);
                else if (e.key === "Escape") setOpen(false);
              }}
              placeholder={t("settings.general.language.searchPlaceholder")}
              className="w-full rounded-md border border-mid-gray/40 bg-mid-gray/10 px-2 py-1 text-sm focus:outline-none focus:ring-1 focus:ring-logo-primary"
            />
          </div>
          <div className="max-h-48 overflow-y-auto">
            <button
              type="button"
              onClick={() => pick("all")}
              className="w-full px-3 py-1.5 text-left text-sm hover:bg-mid-gray/10"
            >
              {t("settings.models.filters.allLanguages")}
            </button>
            {langs.map((l) => (
              <button
                key={l.value}
                type="button"
                onClick={() => pick(l.value)}
                className={`w-full px-3 py-1.5 text-left text-sm ${
                  value === l.value
                    ? "bg-logo-primary/20 font-semibold text-logo-primary"
                    : "hover:bg-mid-gray/10"
                }`}
              >
                {l.label}
              </button>
            ))}
            {langs.length === 0 && (
              <div className="px-3 py-2 text-center text-sm text-text/50">
                {t("settings.general.language.noResults")}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default DictationSection;
