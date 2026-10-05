import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, FolderPlus, Plus, RefreshCw, X } from "lucide-react";
import { commands } from "@/bindings";
import { Button } from "../../ui/Button";
import { useSettings } from "@/hooks/useSettings";
import { useLlmLocalStore } from "@/stores/llmLocalStore";

interface LlmModelDirsProps {
  /** Wie viele Modelle die Ordner zusammen liefern. */
  foundCount: number;
}

/**
 * Modellordner: Ollama-Speicher, LM-Studio-Ablage oder ein eigener Ordner.
 * Die App laedt die Modelle von dort, statt sie ein zweites Mal
 * herunterzuladen. Erkannte Ordner werden nur vorgeschlagen, nie von selbst
 * eingetragen -- was die App durchsucht, entscheidet der Nutzer.
 */
export const LlmModelDirs: React.FC<LlmModelDirsProps> = ({ foundCount }) => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const { setModelDirs, rescanModelDirs, rescanning } = useLlmLocalStore();
  const dirs = getSetting("llm_model_dirs") ?? [];
  const [suggestions, setSuggestions] = useState<string[]>([]);

  useEffect(() => {
    void commands
      .llmSuggestModelDirs()
      .then((s) => setSuggestions(s ?? []))
      .catch(() => {});
  }, []);

  const save = async (next: string[]) => {
    if (await setModelDirs(next)) await refreshSettings();
  };

  const add = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) await save([...dirs, picked]);
  };

  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  const openSuggestions = suggestions.filter((s) => !dirs.some((d) => same(d, s)));

  return (
    <div className="flex flex-col px-4 py-3 gap-2" data-testid="llm-model-dirs">
      <div className="flex items-start justify-between gap-3 flex-wrap">
        <div className="min-w-0 flex-1">
          <h3 className="text-base font-semibold text-text">
            {t("settings.models.llm.dirs.title")}
          </h3>
          <p className="text-text/60 text-sm leading-relaxed">
            {t("settings.models.llm.dirs.description")}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {dirs.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void rescanModelDirs()}
              disabled={rescanning}
              className="flex items-center gap-1.5"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${rescanning ? "animate-spin" : ""}`} />
              <span>{t("settings.models.llm.dirs.rescan")}</span>
            </Button>
          )}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void add()}
            disabled={rescanning}
            className="flex items-center gap-1.5"
          >
            <FolderPlus className="w-3.5 h-3.5" />
            <span>{t("settings.models.llm.dirs.add")}</span>
          </Button>
        </div>
      </div>
      {dirs.length === 0 ? (
        <p className="text-xs text-text/50">{t("settings.models.llm.dirs.empty")}</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {dirs.map((dir) => (
            <li
              key={dir}
              className="flex items-center gap-2 text-sm text-text/80 min-w-0"
              data-model-dir={dir}
            >
              <FolderOpen className="w-3.5 h-3.5 shrink-0 text-text/50" />
              <span className="truncate flex-1 min-w-0" title={dir}>
                {dir}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void save(dirs.filter((d) => d !== dir))}
                disabled={rescanning}
                aria-label={t("settings.models.llm.dirs.remove")}
                title={t("settings.models.llm.dirs.remove")}
              >
                <X className="w-3.5 h-3.5" />
              </Button>
            </li>
          ))}
          <li className="text-xs text-text/50">
            {t("settings.models.llm.dirs.found", { count: foundCount })}
          </li>
        </ul>
      )}
      {openSuggestions.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {openSuggestions.map((s) => (
            <button
              key={s}
              type="button"
              onClick={() => void save([...dirs, s])}
              disabled={rescanning}
              className="flex items-center gap-1 text-xs px-2 py-1 rounded-md border border-mid-gray/40 text-text/70 hover:bg-mid-gray/10 hover:text-text disabled:opacity-50"
              data-suggested-dir={s}
            >
              <Plus className="w-3 h-3" />
              {t("settings.models.llm.dirs.suggest", { path: s })}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};

export default LlmModelDirs;
