import React from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Download,
  Globe,
  HardDrive,
  Loader2,
  Trash2,
  Wrench,
} from "lucide-react";
import type { TtsDownloadInfo } from "@/bindings";
import { formatModelSize } from "@/lib/utils/format";
import { getLanguageLabel } from "@/lib/constants/languages";
import Badge from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";

// Piper runtime/voice row (Paket B-E3). Visually in the same family as
// `ModelCard` (same badges, buttons, progress-bar treatment) but not a
// re-skin of it: a Piper voice has no "select to activate" click — only
// explicit Download/Cancel/Delete actions — so whole-row click-to-select
// would misrepresent an affordance that doesn't exist yet (voice selection
// is a parallel package's job).
interface TtsVoiceCardProps {
  info: TtsDownloadInfo;
  onDownload: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  downloadProgress?: number;
  /** Live frontend state, not `info.is_downloading` — the backend snapshot in
   *  `info` is only refreshed on load/complete/delete, not on every progress
   *  tick (same reasoning as `ModelCard`'s parent-computed `status` prop). */
  isDownloading?: boolean;
  isVerifying?: boolean;
  /** Das Piper-Programm nachladen/reparieren (nur fuer Stimmen, deren Dateien
   *  da sind, deren Laufzeit aber fehlt oder unvollstaendig ist). */
  onInstallRuntime?: () => void;
}

export const TtsVoiceCard: React.FC<TtsVoiceCardProps> = ({
  info,
  onDownload,
  onCancel,
  onDelete,
  downloadProgress,
  isDownloading = false,
  isVerifying = false,
  onInstallRuntime,
}) => {
  const { t } = useTranslation();
  const isRuntime = info.kind === "runtime";

  const nameKey = isRuntime
    ? "settings.models.ttsVoices.runtime.name"
    : `settings.models.ttsVoices.voices.${info.id}.name`;
  const descriptionKey = isRuntime
    ? "settings.models.ttsVoices.runtime.description"
    : `settings.models.ttsVoices.voices.${info.id}.description`;
  const displayName = t(nameKey, { defaultValue: info.name });
  const displayDescription = t(descriptionKey, { defaultValue: info.description });
  const languageLabel = info.language ? getLanguageLabel(info.language) : null;
  // Issue #29: eine Stimme zaehlt erst als nutzbar, wenn auch das Programm da
  // ist; sonst wird sie als "nicht nutzbar" markiert statt erst beim
  // Vorlesen zu scheitern.
  const unsupported = info.unsupported_reason ?? null;
  const needsRuntime = !isRuntime && info.is_downloaded && !info.is_usable;
  const license = info.license ?? null;

  return (
    <div className="flex flex-col px-4 py-3 gap-2">
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <div className="flex items-center gap-2 flex-wrap">
          <h3 className="text-base font-semibold text-text">{displayName}</h3>
          {info.is_downloaded && !needsRuntime && !unsupported && (
            <Badge variant="success">
              {t("settings.models.ttsVoices.status.installed")}
            </Badge>
          )}
          {needsRuntime && (
            <Badge variant="warning">
              {t("settings.models.ttsVoices.status.runtimeMissing")}
            </Badge>
          )}
          {unsupported && (
            <Badge variant="warning">
              {t("settings.models.ttsVoices.status.unsupported")}
            </Badge>
          )}
          {info.license_non_commercial && (
            <Badge variant="warning">
              {t("settings.models.ttsVoices.license.nonCommercial")}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-2">
          {needsRuntime && onInstallRuntime && !isDownloading && (
            <Button
              variant="secondary"
              size="sm"
              onClick={onInstallRuntime}
              className="flex items-center gap-1.5"
            >
              <Wrench className="w-3.5 h-3.5" />
              <span>{t("settings.models.ttsVoices.actions.installRuntime")}</span>
            </Button>
          )}
          {!info.is_downloaded && !isDownloading && !unsupported && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onDownload(info.id)}
              className="flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5" />
              <span>{t("settings.models.ttsVoices.actions.download")}</span>
            </Button>
          )}
          {info.is_downloaded && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onDelete(info.id)}
              title={t("settings.models.ttsVoices.actions.delete")}
              className="flex items-center gap-1.5 text-logo-primary/85 hover:text-logo-primary hover:bg-logo-primary/10"
            >
              <Trash2 className="w-3.5 h-3.5" />
              <span>{t("settings.models.ttsVoices.actions.delete")}</span>
            </Button>
          )}
        </div>
      </div>

      <p className="text-text/60 text-sm leading-relaxed">{displayDescription}</p>
      {unsupported && (
        <p
          className="text-amber-700 dark:text-amber-400 text-sm leading-relaxed"
          data-testid="tts-unsupported-note"
        >
          {t(`settings.models.ttsVoices.unsupported.${unsupported}`, {
            defaultValue: unsupported,
          })}
        </p>
      )}
      {info.license_non_commercial && (
        <p
          className="text-amber-700 dark:text-amber-400 text-sm leading-relaxed"
          data-testid="tts-nc-note"
        >
          {t(`settings.models.ttsVoices.voices.${info.id}.nonCommercialHint`, {
            // Von einer Forschungs-/NC-Stimme abgeleitete Stimmen erklaeren ihre Herkunft.
            defaultValue: t("settings.models.ttsVoices.license.nonCommercialHint", {
              license: license ?? "",
            }),
          })}
        </p>
      )}

      <div className="flex items-center gap-3 text-xs text-text/50">
        {languageLabel && (
          <div className="flex items-center gap-1">
            <Globe className="w-3.5 h-3.5" />
            <span>{languageLabel}</span>
          </div>
        )}
        {license && (
          <span className="flex items-center gap-1.5">
            <span>
              {t("settings.models.ttsVoices.license.label", { license })}
            </span>
            {info.license_url && (
              <button
                type="button"
                onClick={() => void openUrl(info.license_url as string)}
                className="underline hover:text-text cursor-pointer"
              >
                {t("settings.models.ttsVoices.license.source")}
              </button>
            )}
          </span>
        )}
        {!isDownloading && (
          <span className="flex items-center gap-1.5 ms-auto">
            <HardDrive className="w-3.5 h-3.5" />
            <span>{formatModelSize(info.size_mb)}</span>
          </span>
        )}
      </div>

      {isDownloading && !isVerifying && downloadProgress !== undefined && (
        <div className="w-full mt-1">
          <div className="w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
            <div
              className="h-full bg-logo-primary rounded-full transition-all duration-300"
              style={{ width: `${downloadProgress}%` }}
            />
          </div>
          <div className="flex items-center justify-between text-xs mt-1">
            <span className="text-text/50">
              {t("settings.models.ttsVoices.status.downloading", {
                percentage: Math.round(downloadProgress),
              })}
            </span>
            <Button
              variant="danger-ghost"
              size="sm"
              onClick={() => onCancel(info.id)}
              aria-label={t("settings.models.ttsVoices.actions.cancel")}
            >
              {t("settings.models.ttsVoices.actions.cancel")}
            </Button>
          </div>
        </div>
      )}
      {isDownloading && isVerifying && (
        <div className="w-full mt-1">
          <div className="w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
            <div className="h-full bg-logo-primary rounded-full animate-pulse w-full" />
          </div>
          <p className="text-xs text-text/50 mt-1 flex items-center gap-1.5">
            <Loader2 className="w-3 h-3 animate-spin" />
            {t("modelSelector.verifyingGeneric")}
          </p>
        </div>
      )}
    </div>
  );
};

export default TtsVoiceCard;
