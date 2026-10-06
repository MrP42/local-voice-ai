import React from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Download, Globe, Trash2, Wrench } from "lucide-react";
import type { TtsDownloadInfo } from "@/bindings";
import { useTtsModelStore } from "@/stores/ttsModelStore";
import { formatModelSize } from "@/lib/utils/format";
import { getLanguageLabel } from "@/lib/constants/languages.ts";
import { Button } from "../../ui/Button";
import type { ActionMenuItem } from "../../ui/ActionMenu";
import { ModelRow, RowChip, type RowProgress } from "./ModelRow";
import { ModelSection } from "./ModelSection";

/**
 * Vorlesestimmen (Piper): installierte Stimmen als Zeilen, ladbare zugeklappt.
 * Das Piper-Programm selbst steht bei den installierten, solange es fehlt
 * oder repariert werden muss.
 */
export const VoicesSection: React.FC<{ query: string }> = ({ query }) => {
  const { t } = useTranslation();
  const {
    downloads,
    downloadingIds,
    verifyingIds,
    downloadProgress,
    downloadModel,
    cancelDownload,
    deleteModel,
  } = useTtsModelStore();

  const nameOf = (info: TtsDownloadInfo) =>
    t(
      info.kind === "runtime"
        ? "settings.models.ttsVoices.runtime.name"
        : `settings.models.ttsVoices.voices.${info.id}.name`,
      { defaultValue: info.name },
    );
  const descriptionOf = (info: TtsDownloadInfo) =>
    t(
      info.kind === "runtime"
        ? "settings.models.ttsVoices.runtime.description"
        : `settings.models.ttsVoices.voices.${info.id}.description`,
      { defaultValue: info.description },
    );

  const runtime = downloads.find(
    (d) => d.kind === "runtime" && !d.unsupported_reason,
  );

  const remove = async (info: TtsDownloadInfo) => {
    const ok = await ask(
      t("settings.models.ttsVoices.deleteConfirm", { name: nameOf(info) }),
      { title: t("settings.models.ttsVoices.deleteTitle"), kind: "warning" },
    );
    if (ok) await deleteModel(info.id).catch(() => {});
  };

  const q = query.trim().toLowerCase();
  const shown = downloads.filter(
    (d) =>
      !q ||
      `${nameOf(d)} ${descriptionOf(d)} ${d.language ?? ""}`
        .toLowerCase()
        .includes(q),
  );
  const installed = shown.filter(
    (d) => d.is_downloaded || d.id in downloadingIds,
  );
  const available = shown.filter(
    (d) => !d.is_downloaded && !(d.id in downloadingIds),
  );

  const row = (info: TtsDownloadInfo) => {
    const isRuntime = info.kind === "runtime";
    const downloading = info.id in downloadingIds;
    const unsupported = info.unsupported_reason ?? null;
    // Eine Stimme ist erst nutzbar, wenn auch das Programm da ist (#29).
    const needsRuntime = !isRuntime && info.is_downloaded && !info.is_usable;
    const language = info.language ? getLanguageLabel(info.language) : null;
    const menu: ActionMenuItem[] = info.is_downloaded
      ? [
          {
            id: "delete",
            label: t("settings.models.ttsVoices.actions.delete"),
            icon: Trash2,
            onSelect: () => void remove(info),
          },
        ]
      : [];
    const progress: RowProgress | null =
      downloading && info.id in verifyingIds
        ? { kind: "busy", label: t("modelSelector.verifyingGeneric") }
        : downloading
          ? {
              kind: "download",
              percent: downloadProgress[info.id]?.percentage ?? 0,
              onCancel: () => void cancelDownload(info.id),
            }
          : null;
    return (
      <ModelRow
        key={info.id}
        name={nameOf(info)}
        dimmed={!!unsupported}
        dataAttrs={{ "data-tts-download": info.id }}
        badges={
          <>
            {needsRuntime && (
              <RowChip tone="warning">
                {t("settings.models.ttsVoices.status.runtimeMissing")}
              </RowChip>
            )}
            {unsupported && (
              <RowChip tone="warning">
                {t("settings.models.ttsVoices.status.unsupported")}
              </RowChip>
            )}
            {info.license_non_commercial && (
              <RowChip tone="warning">
                {t("settings.models.ttsVoices.license.nonCommercial")}
              </RowChip>
            )}
          </>
        }
        meta={
          <>
            {language && (
              <span className="flex items-center gap-1">
                <Globe className="h-3.5 w-3.5" />
                {language}
              </span>
            )}
            <span className="tabular-nums">
              {formatModelSize(info.size_mb)}
            </span>
          </>
        }
        primary={
          needsRuntime && runtime && !downloading ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void downloadModel(runtime.id)}
              className="flex items-center gap-1.5"
            >
              <Wrench className="h-3.5 w-3.5" />
              {t("settings.models.ttsVoices.actions.installRuntime")}
            </Button>
          ) : !info.is_downloaded && !downloading && !unsupported ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void downloadModel(info.id)}
              className="flex items-center gap-1.5"
            >
              <Download className="h-3.5 w-3.5" />
              {t("settings.models.ttsVoices.actions.download")}
            </Button>
          ) : null
        }
        menu={menu}
        progress={progress}
        details={
          <>
            <p>{descriptionOf(info)}</p>
            {unsupported && (
              <p
                className="text-amber-700 dark:text-amber-400"
                data-testid="tts-unsupported-note"
              >
                {t(`settings.models.ttsVoices.unsupported.${unsupported}`, {
                  defaultValue: unsupported,
                })}
              </p>
            )}
            {info.license_non_commercial && (
              <p
                className="text-amber-700 dark:text-amber-400"
                data-testid="tts-nc-note"
              >
                {t(
                  `settings.models.ttsVoices.voices.${info.id}.nonCommercialHint`,
                  {
                    defaultValue: t(
                      "settings.models.ttsVoices.license.nonCommercialHint",
                      { license: info.license ?? "" },
                    ),
                  },
                )}
              </p>
            )}
            {info.license && (
              <p className="text-xs text-text/50">
                {t("settings.models.ttsVoices.license.label", {
                  license: info.license,
                })}{" "}
                {info.license_url && (
                  <button
                    type="button"
                    onClick={() => void openUrl(info.license_url as string)}
                    className="underline hover:text-text"
                  >
                    {t("settings.models.ttsVoices.license.source")}
                  </button>
                )}
              </p>
            )}
          </>
        }
      />
    );
  };

  return (
    <div className="space-y-5" data-models-section="voices">
      <p className="text-xs text-text/50">
        {t("settings.models.ttsVoices.description")}
      </p>
      <ModelSection
        id="tts-installed"
        title={t("settings.models.groups.installed")}
        count={installed.length}
        collapsible={false}
      >
        {installed.length > 0 ? (
          installed.map(row)
        ) : (
          <p className="px-3 py-3 text-sm text-text/50">
            {t("settings.models.groups.installedEmpty")}
          </p>
        )}
      </ModelSection>
      {available.length > 0 && (
        <ModelSection
          id="tts-available"
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

export default VoicesSection;
