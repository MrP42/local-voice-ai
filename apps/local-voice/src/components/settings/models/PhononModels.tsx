import React from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Download, Globe, Trash2, Wrench } from "lucide-react";
import type { TtsDownloadInfo } from "@/bindings";
import { useTtsModelStore } from "@/stores/ttsModelStore";
import { getLanguageLabel } from "@/lib/constants/languages.ts";
import { Button } from "../../ui/Button";
import type { ActionMenuItem } from "../../ui/ActionMenu";
import { ModelRow, RowChip, type RowProgress } from "./ModelRow";
import { ModelSection } from "./ModelSection";

/** Gehoert die Zeile zu Phonon? Zeilen ohne `engine` (Piper) nicht. */
export const isPhononRow = (info: TtsDownloadInfo) => info.engine === "phonon";

/** Bytes lesbar: Stimmen sind einige hundert KB gross, Modelle zig MB. */
export const formatBytes = (bytes: number): string => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "?";
  if (bytes >= 1024 * 1024) {
    const mb = bytes / (1024 * 1024);
    return `${new Intl.NumberFormat(undefined, {
      maximumFractionDigits: mb >= 100 ? 0 : 1,
    }).format(mb)} MB`;
  }
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
};

/** `hf://OWNER/REPO@COMMIT` -> `OWNER/REPO`; sonst null. */
const hfRepo = (source: string | null | undefined): string | null =>
  source?.match(/^hf:\/\/([^@]+)/)?.[1] ?? null;

/**
 * Phonon (experimentell): Programm `ptts` und Modelle auf der Modellseite
 * installieren und entfernen -- wie Piper. Die Stimmen der Sprache gehoeren
 * zum Modell (laden mit, loeschen mit, in der Groesse enthalten) und stehen
 * hier nicht als eigene Zeilen. Fehlen bei einem installierten Modell Stimmen
 * (aelterer Stand), bietet seine Zeile "Stimmen nachladen" an. Der Katalog
 * liefert Groesse, SHA-256 und Lizenz; das Backend prueft jede Datei.
 */
export const PhononModels: React.FC<{ query: string }> = ({ query }) => {
  const { t } = useTranslation();
  const {
    downloads: all,
    downloadingIds,
    verifyingIds,
    downloadProgress,
    downloadModel,
    cancelDownload,
    deleteModel,
  } = useTtsModelStore();
  const downloads = all.filter(isPhononRow);
  const byId = new Map(downloads.map((d) => [d.id, d]));

  const nameOf = (info: TtsDownloadInfo) =>
    t(`settings.models.phonon.items.${info.id}.name`, {
      defaultValue: info.name,
    });
  const descriptionOf = (info: TtsDownloadInfo) =>
    t(`settings.models.phonon.items.${info.id}.description`, {
      defaultValue: info.description,
    });

  /** Die erste fehlende Voraussetzung (Stimme -> Modell -> Programm). */
  const prerequisite = (info: TtsDownloadInfo): TtsDownloadInfo | null => {
    const seen = new Set<string>([info.id]);
    let cur = info.requires ? byId.get(info.requires) : undefined;
    while (cur && !seen.has(cur.id)) {
      if (!cur.is_downloaded) return cur;
      seen.add(cur.id);
      cur = cur.requires ? byId.get(cur.requires) : undefined;
    }
    return null;
  };

  const remove = async (info: TtsDownloadInfo) => {
    const ok = await ask(
      t(`settings.models.phonon.deleteConfirm.${info.kind}`, {
        name: nameOf(info),
      }),
      { title: t("settings.models.ttsVoices.deleteTitle"), kind: "warning" },
    );
    if (ok) await deleteModel(info.id).catch(() => {});
  };

  const q = query.trim().toLowerCase();
  const shown = downloads.filter(
    (d) =>
      !q ||
      `${nameOf(d)} ${descriptionOf(d)} ${d.language ?? ""} phonon`
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
    const downloading = info.id in downloadingIds;
    const unsupported = info.unsupported_reason ?? null;
    const missing = prerequisite(info);
    const needsProgram =
      info.is_downloaded && !info.is_usable && missing?.kind === "runtime";
    const language = info.language ? getLanguageLabel(info.language) : null;
    const repo = hfRepo(info.source);
    // Stimmen des Modells (aeltere Mocks ohne die Felder: keine).
    const voicesTotal = info.bundled_voices ?? 0;
    const voicesPresent = info.bundled_voices_present ?? 0;
    const voicesMissing =
      info.is_downloaded && voicesTotal > 0 && voicesPresent < voicesTotal;
    const menu: ActionMenuItem[] = info.is_downloaded
      ? [
          {
            id: "delete",
            testId: "row-delete",
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
    const install = (target: TtsDownloadInfo, label: string) => (
      <Button
        variant="secondary"
        size="sm"
        onClick={() => void downloadModel(target.id)}
        className="flex items-center gap-1.5"
        data-testid={`phonon-install-${target.id}`}
      >
        <Wrench className="h-3.5 w-3.5" />
        {label}
      </Button>
    );
    return (
      <ModelRow
        key={info.id}
        name={nameOf(info)}
        dimmed={!!unsupported}
        dataAttrs={{ "data-tts-download": info.id, "data-engine": "phonon" }}
        badges={
          <>
            {needsProgram && (
              <RowChip tone="warning">
                {t("settings.models.ttsVoices.status.runtimeMissing")}
              </RowChip>
            )}
            {voicesMissing && (
              <RowChip
                tone="warning"
                title={t("settings.models.phonon.voices.detail", {
                  total: voicesTotal,
                  present: voicesPresent,
                })}
              >
                {t("settings.models.phonon.voices.missingChip", {
                  present: voicesPresent,
                  total: voicesTotal,
                })}
              </RowChip>
            )}
            {unsupported && (
              <RowChip tone="warning">
                {t("settings.models.ttsVoices.status.unsupported")}
              </RowChip>
            )}
            {info.gated && (
              <RowChip
                tone="neutral"
                title={t("settings.models.phonon.gated.title")}
              >
                {t("settings.models.phonon.gated.chip")}
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
            <span className="tabular-nums">{formatBytes(info.size_bytes)}</span>
            {voicesTotal > 0 && (
              <span data-testid={`phonon-voices-count-${info.id}`}>
                {t("settings.models.phonon.voices.included", {
                  count: voicesTotal,
                })}
              </span>
            )}
          </>
        }
        primary={
          missing && !downloading ? (
            // Wird die Voraussetzung schon geladen, zeigt deren Zeile den Fortschritt.
            missing.id in downloadingIds ? null : (
              install(
                missing,
                missing.kind === "runtime"
                  ? t("settings.models.phonon.actions.installRuntime")
                  : t("settings.models.phonon.actions.installModel"),
              )
            )
          ) : voicesMissing && !downloading ? (
            // Das Modell bleibt, wie es ist: geladen werden nur die fehlenden Stimmen.
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void downloadModel(info.id)}
              className="flex items-center gap-1.5"
              data-testid={`phonon-voices-reload-${info.id}`}
            >
              <Download className="h-3.5 w-3.5" />
              {t("settings.models.phonon.voices.reload")}
            </Button>
          ) : !info.is_downloaded && !downloading && !unsupported ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void downloadModel(info.id)}
              className="flex items-center gap-1.5"
              data-testid={`phonon-install-${info.id}`}
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
            {voicesTotal > 0 && (
              <p data-testid={`phonon-voices-detail-${info.id}`}>
                {info.is_downloaded
                  ? t("settings.models.phonon.voices.detail", {
                      total: voicesTotal,
                      present: voicesPresent,
                    })
                  : t("settings.models.phonon.voices.detailNew", {
                      total: voicesTotal,
                    })}
              </p>
            )}
            {info.license_non_commercial && (
              <p
                className="text-amber-700 dark:text-amber-400"
                data-testid="tts-nc-note"
              >
                {t(`settings.models.phonon.nonCommercial.${info.kind}`, {
                  license: info.license ?? "",
                })}
              </p>
            )}
            {info.gated && (
              <p
                className="text-amber-700 dark:text-amber-400"
                data-testid="phonon-gated-note"
              >
                {t("settings.models.phonon.gated.hint", {
                  repo: repo ?? info.name,
                })}
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
            {info.license_note && (
              <p className="text-xs text-text/50">{info.license_note}</p>
            )}
            {info.source && (
              <p
                className="break-all text-xs text-text/50"
                data-testid="phonon-source"
              >
                {t("settings.models.phonon.source", { source: info.source })}
              </p>
            )}
            {info.sha256 ? (
              <p
                className="break-all font-mono text-xs text-text/50"
                data-testid="phonon-sha256"
              >
                {t("settings.models.phonon.sha256", { sha: info.sha256 })}
              </p>
            ) : (
              <p className="text-xs text-text/50">
                {t("settings.models.phonon.sha256Files")}
              </p>
            )}
          </>
        }
      />
    );
  };

  return (
    <div className="space-y-5" data-models-section="phonon">
      <div className="space-y-1">
        <h3 className="text-sm font-semibold text-text">
          {t("settings.models.phonon.title")}
        </h3>
        <p className="text-xs text-text/50">
          {t("settings.models.phonon.description")}
        </p>
      </div>
      <ModelSection
        id="phonon-installed"
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
          id="phonon-available"
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

export default PhononModels;
