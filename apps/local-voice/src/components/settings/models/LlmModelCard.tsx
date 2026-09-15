import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Cpu, Download, HardDrive, Loader2, Trash2 } from "lucide-react";
import { commands, type FitReport, type LlmDownloadInfo } from "@/bindings";
import { Button } from "../../ui/Button";
import Badge from "../../ui/Badge";
import { formatModelSize } from "@/lib/utils/format";

interface LlmModelCardProps {
  info: LlmDownloadInfo;
  /** Ist dieses Modell das aktive Sprachmodell der App? */
  isActive?: boolean;
  /** Bedient der lokale Server dieses Modell gerade (geladen im Speicher)? */
  isServing?: boolean;
  onDownload: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  onActivate?: (id: string) => void;
  downloadProgress?: number;
  isDownloading?: boolean;
  isVerifying?: boolean;
}

/**
 * Eine Karte für ein Laufzeitpaket oder ein Sprachmodell.
 *
 * Merkmale ("schnell", "empfohlen", "zusammenfassung") kommen aus dem
 * Katalog und sagen, wofür ein Modell taugt — keine erfundenen Zahlen. Bei
 * Laufzeiten steht stattdessen das Backend, damit klar ist, was man
 * installiert: Vulkan läuft überall, CUDA nur auf NVIDIA.
 */
export const LlmModelCard: React.FC<LlmModelCardProps> = ({
  info,
  isActive = false,
  isServing = false,
  onDownload,
  onCancel,
  onDelete,
  onActivate,
  downloadProgress,
  isDownloading = false,
  isVerifying = false,
}) => {
  const { t } = useTranslation();
  const isRuntime = info.kind === "runtime";
  const tags: string[] = isRuntime ? [] : info.tags;

  // Passt es rein? Einmal je Karte, gegen das aktuell freie Budget. Ohne
  // Backend (Browser-Test) bleibt das Feld leer -- lieber nichts als Zahlen,
  // die niemand gemessen hat.
  const [fit, setFit] = useState<FitReport | null>(null);
  useEffect(() => {
    if (isRuntime) return;
    let cancelled = false;
    void commands
      .llmLocalFit(info.id, null)
      .then((result) => {
        if (!cancelled && result.status === "ok") setFit(result.data);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [info.id, isRuntime]);
  const gb = (mb: number) => (mb / 1024).toFixed(1).replace(".", ",");

  return (
    <div
      className="flex flex-col px-4 py-3 gap-2"
      data-llm-card={info.id}
      data-active={isActive || undefined}
    >
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <div className="flex items-center gap-2 flex-wrap">
          <h3 className="text-base font-semibold text-text">{info.name}</h3>
          {info.is_downloaded && (
            <Badge variant="success">{t("settings.models.llm.status.installed")}</Badge>
          )}
          {isActive && <Badge variant="primary">{t("settings.models.llm.status.active")}</Badge>}
          {isServing && (
            <Badge variant="secondary">{t("settings.models.llm.status.loaded")}</Badge>
          )}
          {isRuntime && info.backend && (
            <Badge variant="secondary">
              {t(`settings.models.llm.backend.${info.backend}`, {
                defaultValue: info.backend,
              })}
            </Badge>
          )}
          {!isRuntime && <Badge variant="secondary">{t(info.for_this_platform ? "localModels.compatible" : "localModels.unsupported")}</Badge>}
          {tags.map((tag) => (
            <Badge key={tag} variant="secondary">
              {t(`settings.models.llm.tags.${tag}`, { defaultValue: tag })}
            </Badge>
          ))}
        </div>
        <div className="flex items-center gap-2">
          {!info.is_downloaded && !isDownloading && info.for_this_platform && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onDownload(info.id)}
              className="flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5" />
              <span>{t("settings.models.llm.actions.download")}</span>
            </Button>
          )}
          {info.is_downloaded && !isRuntime && info.for_this_platform && !isActive && onActivate && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => onActivate(info.id)}
            >
              {t("settings.models.llm.actions.use")}
            </Button>
          )}
          {info.is_downloaded && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onDelete(info.id)}
              title={t("settings.models.llm.actions.delete")}
              className="flex items-center gap-1.5 text-logo-primary/85 hover:text-logo-primary hover:bg-logo-primary/10"
            >
              <Trash2 className="w-3.5 h-3.5" />
              <span>{t("settings.models.llm.actions.delete")}</span>
            </Button>
          )}
        </div>
      </div>
      <p className="text-text/60 text-sm leading-relaxed">{info.description}</p>
      {fit && (
        <p
          className={`text-xs ${
            fit.verdict === "fits"
              ? "text-green-600"
              : fit.verdict === "tight"
                ? "text-yellow-600"
                : fit.verdict === "unlikely"
                  ? "text-red-500"
                  : "text-text/50"
          }`}
          data-fit={fit.verdict}
        >
          {t(fit.on_gpu ? `settings.models.llm.fit.${fit.verdict}` : "localModels.ramEstimate", {
            need: gb(fit.estimate.total_mb),
            free: gb(fit.free_mb),
          })}
          {!fit.estimate.from_metadata && ` ${t("settings.models.llm.fit.rough")}`}
        </p>
      )}
      <div className="flex items-center gap-3 text-xs text-text/50">
        {isRuntime && (
          <span className="flex items-center gap-1">
            <Cpu className="w-3.5 h-3.5" />
            <span>
              {info.for_this_platform
                ? t("settings.models.llm.runtime.forThisMachine")
                : t("settings.models.llm.runtime.otherPlatform")}
            </span>
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
              {t("settings.models.llm.status.downloading", {
                percentage: Math.round(downloadProgress),
              })}
            </span>
            <Button
              variant="danger-ghost"
              size="sm"
              onClick={() => onCancel(info.id)}
              aria-label={t("settings.models.llm.actions.cancel")}
            >
              {t("settings.models.llm.actions.cancel")}
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

export default LlmModelCard;
