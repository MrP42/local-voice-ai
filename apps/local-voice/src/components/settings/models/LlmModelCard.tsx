import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
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
  /** Nur Modelle aus Modellordnern: Ladeversuch starten. */
  onProbe?: (id: string) => void;
  isProbing?: boolean;
  /** Nur Katalogmodelle mit geprueftem Ersatz: App-Kopie loeschen. */
  onDeleteCopy?: (id: string) => void;
  downloadProgress?: number;
  isDownloading?: boolean;
  isVerifying?: boolean;
}

/** Klartext fuer den Grund, warum ein fremdes Modell nicht laedt. */
const reasonText = (t: TFunction, reason: string) =>
  reason === "ollama_merged_vision" || reason === "server_exited"
    ? t(`settings.models.llm.external.reasons.${reason}`)
    : t("settings.models.llm.external.reasons.generic", { reason });

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
  onProbe,
  isProbing = false,
  onDeleteCopy,
  downloadProgress,
  isDownloading = false,
  isVerifying = false,
}) => {
  const { t } = useTranslation();
  const isRuntime = info.kind === "runtime";
  const tags: string[] = isRuntime ? [] : info.tags;
  const external = info.external;
  const compat = external?.compat.state;
  const incompatible =
    external?.compat.state === "incompatible" ? external.compat : null;

  // Passt es rein? Einmal je Karte, gegen das aktuell freie Budget. Ohne
  // Backend (Browser-Test) bleibt das Feld leer -- lieber nichts als Zahlen,
  // die niemand gemessen hat.
  const [fit, setFit] = useState<FitReport | null>(null);
  useEffect(() => {
    // Ein Modell, das sicher nicht laedt, braucht keine Speicherprognose.
    if (isRuntime || incompatible) return;
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
  }, [info.id, isRuntime, incompatible]);
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
          {external ? (
            <>
              <Badge variant="secondary">
                {t(`settings.models.llm.external.source.${external.source}`)}
              </Badge>
              {/* Badge reicht keine data-Attribute durch -- daher die Huelle. */}
              <span data-compat={compat} className="inline-flex">
                <Badge
                  variant={
                    compat === "ok"
                      ? "success"
                      : compat === "incompatible"
                        ? "warning"
                        : "secondary"
                  }
                >
                  {t(`settings.models.llm.external.compat.${compat}`)}
                </Badge>
              </span>
            </>
          ) : (
            info.is_downloaded && (
              <Badge variant="success">
                {t("settings.models.llm.status.installed")}
              </Badge>
            )
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
          {external && compat === "unchecked" && onProbe && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onProbe(info.id)}
              disabled={isProbing}
              title={t("settings.models.llm.external.probeHint")}
              className="flex items-center gap-1.5"
            >
              {isProbing && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
              <span>
                {isProbing
                  ? t("settings.models.llm.external.probing")
                  : t("settings.models.llm.external.probe")}
              </span>
            </Button>
          )}
          {info.is_downloaded &&
            !isRuntime &&
            !isActive &&
            onActivate &&
            !incompatible && (
              <Button
                variant="primary"
                size="sm"
                onClick={() => onActivate(info.id)}
              >
                {t("settings.models.llm.actions.use")}
              </Button>
            )}
          {/* Fremde Dateien gehoeren Ollama, LM Studio oder dem Nutzer --
              ohne Entfernen-Knopf. */}
          {info.is_downloaded && !external && (
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
      {external ? (
        <p className="text-text/50 text-xs break-all" title={external.path}>
          {external.path}
        </p>
      ) : (
        <p className="text-text/60 text-sm leading-relaxed">
          {info.description}
        </p>
      )}
      {incompatible && (
        <p
          className="text-xs text-red-500"
          role="note"
          data-incompatible-reason
        >
          {reasonText(t, incompatible.reason)}
        </p>
      )}
      {info.replaceable_by && onDeleteCopy && (
        <div
          className="flex items-center justify-between gap-2 flex-wrap rounded-md bg-mid-gray/10 px-3 py-2"
          data-replaceable-by={info.replaceable_by}
        >
          <p className="text-xs text-text/70 flex-1 min-w-0">
            {t("settings.models.llm.replace.hint", {
              name: info.replaceable_by,
            })}
          </p>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onDeleteCopy(info.id)}
            className="flex items-center gap-1.5"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>
              {t("settings.models.llm.replace.action", {
                size: formatModelSize(info.size_mb),
              })}
            </span>
          </Button>
        </div>
      )}
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
          {t(`settings.models.llm.fit.${fit.verdict}`, {
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
