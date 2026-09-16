import { arch, platform } from "@tauri-apps/plugin-os";
import { useTranslation } from "react-i18next";
import type { LlmDownloadInfo, LocalLlmStatus } from "@/bindings";

export function LocalModelsStatus({
  models,
  runtimes,
  activeId,
  status,
}: {
  models: LlmDownloadInfo[];
  runtimes: LlmDownloadInfo[];
  activeId: string | null;
  status: LocalLlmStatus | null;
}) {
  const { t } = useTranslation();
  const isMac = platform() === "macos";
  const loaded = models.find((m) => m.id === status?.model_id);
  const active = models.find((m) => m.id === activeId);
  const installed = runtimes.filter((r) => r.is_downloaded);
  return (
    <div
      className="rounded-lg border border-mid-gray/30 px-4 py-3 space-y-1 text-sm"
      data-testid="local-models-status"
    >
      <h3 className="font-semibold">
        {t(
          isMac
            ? arch() === "aarch64"
              ? "localModels.macApple"
              : "localModels.macIntel"
            : "localModels.computer",
        )}
      </h3>
      <p className="text-text/70">
        {t("localModels.installed", {
          count: models.filter((m) => m.is_downloaded).length,
          total: models.length,
        })}
      </p>
      <p className="text-text/70">
        {installed.length
          ? t("localModels.runtimeReady", {
              backends: installed
                .map((r) =>
                  t(`settings.models.llm.backend.${r.backend ?? "cpu"}`),
                )
                .join(" · "),
            })
          : t("localModels.runtimeNeeded")}
      </p>
      <p className="text-text/70">
        {active
          ? t("localModels.selected", { name: active.name })
          : t("localModels.notSelected")}
      </p>
      <p className="text-text/80" role="status">
        {status?.phase === "ready"
          ? t("localModels.loaded", {
              name: loaded?.name ?? status.model_id,
              backend: status.backend
                ? t(`settings.models.llm.backend.${status.backend}`, { defaultValue: status.backend })
                : "",
            })
          : status?.phase === "starting"
            ? t("localModels.starting")
            : status?.phase === "error"
              ? t("localModels.error")
              : status
                ? t("localModels.stopped")
                : t("localModels.checking")}
      </p>
      {status?.phase === "error" && status.message && (
        <p className="text-xs text-text/70 break-words">{status.message}</p>
      )}
    </div>
  );
}
