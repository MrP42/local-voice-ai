import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { ArrowLeftRight, Download, FlaskConical, Trash2 } from "lucide-react";
import {
  commands,
  type FitReport,
  type LlmDownloadInfo,
  type LlmModelConfig,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useLlmLocalStore } from "@/stores/llmLocalStore";
import { formatModelSize } from "@/lib/utils/format";
import { Button } from "../../ui/Button";
import type { ActionMenuItem } from "../../ui/ActionMenu";
import { FitDot, ModelRow, RowChip, type RowProgress } from "./ModelRow";
import { ModelSection } from "./ModelSection";
import { LlmModelDirs } from "./LlmModelDirs";
import { ActiveBar } from "./ActiveBar";

/** Klartext fuer den Grund, warum ein fremdes Modell nicht laedt. */
const reasonText = (t: TFunction, reason: string) =>
  reason === "ollama_merged_vision" || reason === "server_exited"
    ? t(`settings.models.llm.external.reasons.${reason}`)
    : t("settings.models.llm.external.reasons.generic", { reason });

/** Woher ein Modell kommt: App, Ollama, LM Studio oder ein Ordner. */
const sourceKey = (info: LlmDownloadInfo) => {
  if (!info.external) return "app";
  if (info.external.source === "ollama") return "ollama";
  return info.external.path.toLowerCase().includes(".lmstudio")
    ? "lmstudio"
    : "folder";
};

const matches = (info: LlmDownloadInfo, q: string) =>
  !q ||
  `${info.name} ${info.description} ${info.external?.path ?? ""} ${info.tags.join(" ")}`
    .toLowerCase()
    .includes(q);

/**
 * Sprachmodelle: oben das aktive, darunter was installiert ist, und
 * zugeklappt Modellordner, Ladbares und die Laufzeit. Ein Modell ist eine
 * Zeile; Beschreibung, Merkmale und Hinweise klappen auf Klick auf.
 */
export const LlmSection: React.FC<{ query: string }> = ({ query }) => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const llm = useLlmLocalStore();
  const { initialize } = llm;
  useEffect(() => {
    void initialize();
  }, [initialize]);

  const q = query.trim().toLowerCase();
  const activeLlmModelId = getSetting("llm_active_model_id") ?? null;
  const localModels = ((getSetting("llm_models") ?? []) as LlmModelConfig[])
    .filter((m) => m.connection_id === "local")
    .map((m) => m.remote_id);
  const activeLocalId =
    ((getSetting("llm_models") ?? []) as LlmModelConfig[]).find(
      (m) => m.id === activeLlmModelId && m.connection_id === "local",
    )?.remote_id ?? null;

  const runtimes = llm.downloads.filter(
    (d) => d.kind === "runtime" && d.for_this_platform,
  );
  const runtimeInstalled = runtimes.some((d) => d.is_downloaded);
  const models = llm.downloads.filter((d) => d.kind === "model");

  // Installiert: was die App selbst geladen hat, plus fremde Modelle, die
  // schon benutzt werden. Der Rest der Modellordner bleibt in seiner Gruppe
  // -- sonst stuenden hier dreissig Ollama-Modelle.
  const { installed, folders, available } = useMemo(() => {
    const installed: LlmDownloadInfo[] = [];
    const folders: LlmDownloadInfo[] = [];
    const available: LlmDownloadInfo[] = [];
    for (const m of models) {
      const downloading = m.id in llm.downloadingIds;
      if (m.external) {
        if (localModels.includes(m.id) || m.id === activeLocalId)
          installed.push(m);
        else folders.push(m);
      } else if (m.is_downloaded || downloading) installed.push(m);
      else available.push(m);
    }
    installed.sort((a, b) =>
      a.id === activeLocalId ? -1 : b.id === activeLocalId ? 1 : 0,
    );
    const rank = (m: LlmDownloadInfo) =>
      ({ ok: 0, unchecked: 1, incompatible: 2 })[
        m.external?.compat.state ?? "unchecked"
      ];
    folders.sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
    return { installed, folders, available };
  }, [models, llm.downloadingIds, localModels.join(","), activeLocalId]);

  const folderBlocked = folders.filter(
    (m) => m.external?.compat.state === "incompatible",
  ).length;

  const activate = (id: string) =>
    void llm.activate(id).then((ok) => {
      if (ok) void refreshSettings();
    });

  const deleteCopy = async (info: LlmDownloadInfo) => {
    if (!info.replaceable_by) return;
    const ok = await ask(
      t("settings.models.llm.replace.confirm", {
        model: info.name,
        name: info.replaceable_by,
      }),
      { kind: "warning" },
    );
    if (ok) await llm.deleteModel(info.id);
  };

  const active = models.find((m) => m.id === activeLocalId) ?? null;
  const serving =
    llm.status?.phase === "ready" ? (llm.status.model_id ?? null) : null;
  const backend =
    llm.status?.backend ??
    runtimes.find((r) => r.is_downloaded)?.backend ??
    null;

  const row = (info: LlmDownloadInfo, withFit: boolean) => (
    <LlmRow
      key={info.id}
      info={info}
      withFit={withFit}
      isActive={info.id === activeLocalId}
      isServing={serving === info.id}
      runtimeInstalled={runtimeInstalled}
      onActivate={activate}
      onDeleteCopy={(i) => void deleteCopy(i)}
    />
  );

  const shownInstalled = installed.filter((m) => matches(m, q));
  const shownFolders = folders.filter((m) => matches(m, q));
  const shownAvailable = available.filter((m) => matches(m, q));

  return (
    <div className="space-y-5" data-models-section="llm">
      <ActiveBar
        label={t("settings.models.active.llm")}
        name={active?.name ?? null}
        chips={
          active && (
            <>
              <RowChip>{t(`settings.models.source.${sourceKey(active)}`)}</RowChip>
              {serving === active.id && (
                <RowChip tone="success">
                  {t("settings.models.llm.status.loaded")}
                </RowChip>
              )}
            </>
          )
        }
        aside={
          runtimeInstalled
            ? t("settings.models.runtime.summary", {
                backend: t(`settings.models.llm.backend.${backend}`, {
                  defaultValue: backend ?? "",
                }),
              })
            : t("settings.models.runtime.missing")
        }
      />

      {llm.error && (
        <p className="text-sm text-red-500 break-words" role="alert">
          {llm.error}
        </p>
      )}

      <ModelSection
        id="llm-installed"
        title={t("settings.models.groups.installed")}
        count={shownInstalled.length}
        collapsible={false}
      >
        {shownInstalled.length > 0 ? (
          shownInstalled.map((m) => row(m, true))
        ) : (
          <p className="px-3 py-3 text-sm text-text/50">
            {t("settings.models.groups.installedEmpty")}
          </p>
        )}
      </ModelSection>

      <ModelSection
        id="llm-folders"
        title={t("settings.models.groups.folders")}
        count={shownFolders.length}
        summary={
          folderBlocked > 0
            ? t("settings.models.groups.foldersBlocked", {
                count: folderBlocked,
              })
            : undefined
        }
        defaultOpen={false}
        forceOpen={q !== ""}
      >
        <div className="border-b border-mid-gray/15">
          <LlmModelDirs
            foundCount={models.filter((m) => m.external).length}
          />
        </div>
        {shownFolders.map((m) => row(m, false))}
      </ModelSection>

      <ModelSection
        id="llm-available"
        title={t("settings.models.groups.available")}
        count={shownAvailable.length}
        defaultOpen={false}
        forceOpen={q !== ""}
      >
        {shownAvailable.map((m) => row(m, true))}
      </ModelSection>

      <ModelSection
        id="llm-runtime"
        title={t("settings.models.groups.runtime")}
        count={runtimes.length}
        defaultOpen={!runtimeInstalled}
      >
        {runtimes.map((r) => (
          <RuntimeRow key={r.id} info={r} />
        ))}
      </ModelSection>
    </div>
  );
};

/** Ein Sprachmodell als Zeile. */
const LlmRow: React.FC<{
  info: LlmDownloadInfo;
  withFit: boolean;
  isActive: boolean;
  isServing: boolean;
  runtimeInstalled: boolean;
  onActivate: (id: string) => void;
  onDeleteCopy: (info: LlmDownloadInfo) => void;
}> = ({
  info,
  withFit,
  isActive,
  isServing,
  runtimeInstalled,
  onActivate,
  onDeleteCopy,
}) => {
  const { t } = useTranslation();
  const llm = useLlmLocalStore();
  const external = info.external;
  const compat = external?.compat.state;
  const incompatible =
    external?.compat.state === "incompatible" ? external.compat : null;
  const downloading = info.id in llm.downloadingIds;
  const verifying = info.id in llm.verifyingIds;
  const probing = info.id in llm.probingIds;

  // Speicherprognose nur, wo sie hilft (installiert, ladbar) -- nicht fuer
  // dreissig Ordnermodelle auf einmal, und nie fuer eines, das nicht laedt.
  const [fit, setFit] = useState<FitReport | null>(null);
  useEffect(() => {
    if (!withFit || incompatible) return;
    let cancelled = false;
    void commands
      .llmLocalFit(info.id, null)
      .then((r) => {
        if (!cancelled && r.status === "ok") setFit(r.data);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [info.id, withFit, incompatible]);
  const gb = (mb: number) => (mb / 1024).toFixed(1).replace(".", ",");
  const fitText = fit
    ? t(`settings.models.llm.fit.${fit.verdict}`, {
        need: gb(fit.estimate.total_mb),
        free: gb(fit.free_mb),
      }) + (fit.estimate.from_metadata ? "" : ` ${t("settings.models.llm.fit.rough")}`)
    : null;

  const canUse =
    (info.is_downloaded || !!external) &&
    !isActive &&
    !incompatible &&
    runtimeInstalled;

  let primary: React.ReactNode = null;
  if (!info.is_downloaded && !external && !downloading) {
    primary = (
      <Button
        variant="secondary"
        size="sm"
        onClick={() => void llm.downloadModel(info.id)}
        className="flex items-center gap-1.5"
      >
        <Download className="h-3.5 w-3.5" />
        {t("settings.models.llm.actions.download")}
      </Button>
    );
  } else if (external && compat === "unchecked" && !canUse) {
    primary = null;
  } else if (canUse) {
    primary = (
      <Button variant="primary" size="sm" onClick={() => onActivate(info.id)}>
        {t("settings.models.llm.actions.use")}
      </Button>
    );
  }

  const menu: ActionMenuItem[] = [];
  if (external && compat === "unchecked") {
    menu.push({
      id: "probe",
      label: probing
        ? t("settings.models.llm.external.probing")
        : t("settings.models.llm.external.probe"),
      icon: FlaskConical,
      title: t("settings.models.llm.external.probeHint"),
      disabled: probing,
      onSelect: () => void llm.probeExternal(info.id),
      testId: "row-probe",
    });
  }
  if (info.replaceable_by) {
    menu.push({
      id: "copy",
      label: t("settings.models.llm.replace.action", {
        size: formatModelSize(info.size_mb),
      }),
      icon: ArrowLeftRight,
      onSelect: () => onDeleteCopy(info),
      testId: "row-delete-copy",
    });
  }
  if (info.is_downloaded && !external) {
    menu.push({
      id: "delete",
      label: t("settings.models.llm.actions.delete"),
      icon: Trash2,
      onSelect: () => void llm.deleteModel(info.id),
      testId: "row-delete",
    });
  }

  const progress: RowProgress | null = verifying
    ? { kind: "busy", label: t("modelSelector.verifyingGeneric") }
    : probing
      ? { kind: "busy", label: t("settings.models.llm.external.probing") }
      : downloading
        ? {
            kind: "download",
            percent: llm.downloadProgress[info.id]?.percentage ?? 0,
            onCancel: () => void llm.cancelDownload(info.id),
          }
        : null;

  return (
    <ModelRow
      name={info.name}
      active={isActive}
      dimmed={!!incompatible}
      dataAttrs={{
        "data-llm-card": info.id,
        "data-active": isActive ? "true" : undefined,
      }}
      badges={
        <>
          {external && (
            <RowChip>{t(`settings.models.source.${sourceKey(info)}`)}</RowChip>
          )}
          {external && (
            <RowChip
              tone={
                compat === "ok"
                  ? "success"
                  : compat === "incompatible"
                    ? "warning"
                    : "neutral"
              }
              dataAttrs={{ "data-compat": compat }}
            >
              {t(`settings.models.llm.external.compat.${compat}`)}
            </RowChip>
          )}
          {isActive && (
            <RowChip tone="accent">{t("settings.models.llm.status.active")}</RowChip>
          )}
          {isServing && (
            <RowChip tone="success">{t("settings.models.llm.status.loaded")}</RowChip>
          )}
          {info.replaceable_by && (
            <RowChip
              title={t("settings.models.llm.replace.hint", {
                name: info.replaceable_by,
              })}
              dataAttrs={{ "data-duplicate": info.replaceable_by }}
            >
              <ArrowLeftRight className="h-3 w-3" />
              {t("settings.models.row.duplicate")}
            </RowChip>
          )}
        </>
      }
      meta={
        <>
          {fit && fitText && <FitDot verdict={fit.verdict} title={fitText} />}
          <span className="tabular-nums">{formatModelSize(info.size_mb)}</span>
        </>
      }
      primary={primary}
      menu={menu}
      progress={progress}
      details={
        <>
          {info.description && <p>{info.description}</p>}
          {info.tags.length > 0 && (
            <p className="text-xs text-text/50">
              {info.tags
                .map((tag) =>
                  t(`settings.models.llm.tags.${tag}`, { defaultValue: tag }),
                )
                .join(" · ")}
            </p>
          )}
          {fitText && (
            <p className="text-xs" data-fit-text={fit?.verdict}>
              {fitText}
            </p>
          )}
          {external && (
            <p className="text-xs text-text/50 break-all">{external.path}</p>
          )}
          {incompatible && (
            <p className="text-xs text-red-500" data-incompatible-reason>
              {reasonText(t, incompatible.reason)}
            </p>
          )}
          {info.replaceable_by && (
            <div
              className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-mid-gray/10 px-3 py-2"
              data-replaceable-by={info.replaceable_by}
            >
              <span className="text-xs">
                {t("settings.models.llm.replace.hint", {
                  name: info.replaceable_by,
                })}
              </span>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onDeleteCopy(info)}
                className="flex items-center gap-1.5"
              >
                <Trash2 className="h-3.5 w-3.5" />
                {t("settings.models.llm.replace.action", {
                  size: formatModelSize(info.size_mb),
                })}
              </Button>
            </div>
          )}
        </>
      }
    />
  );
};

/** Ein Laufzeitpaket (llama-server) als Zeile. */
const RuntimeRow: React.FC<{ info: LlmDownloadInfo }> = ({ info }) => {
  const { t } = useTranslation();
  const llm = useLlmLocalStore();
  const downloading = info.id in llm.downloadingIds;
  return (
    <ModelRow
      name={info.name}
      dataAttrs={{ "data-llm-card": info.id }}
      badges={
        <>
          {info.backend && (
            <RowChip>
              {t(`settings.models.llm.backend.${info.backend}`, {
                defaultValue: info.backend,
              })}
            </RowChip>
          )}
          {info.is_downloaded && (
            <RowChip tone="success">
              {t("settings.models.llm.status.installed")}
            </RowChip>
          )}
        </>
      }
      meta={<span className="tabular-nums">{formatModelSize(info.size_mb)}</span>}
      primary={
        !info.is_downloaded && !downloading ? (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void llm.downloadModel(info.id)}
            className="flex items-center gap-1.5"
          >
            <Download className="h-3.5 w-3.5" />
            {t("settings.models.llm.actions.download")}
          </Button>
        ) : null
      }
      menu={
        info.is_downloaded
          ? [
              {
                id: "delete",
                label: t("settings.models.llm.actions.delete"),
                icon: Trash2,
                onSelect: () => void llm.deleteModel(info.id),
              },
            ]
          : []
      }
      progress={
        downloading
          ? {
              kind: "download",
              percent: llm.downloadProgress[info.id]?.percentage ?? 0,
              onCancel: () => void llm.cancelDownload(info.id),
            }
          : null
      }
      details={info.description ? <p>{info.description}</p> : undefined}
    />
  );
};

export default LlmSection;
