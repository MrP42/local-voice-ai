import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, events, type IndexStatus } from "@/bindings";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/** Katalog-Kennung des Suchmodells (BGE-M3, `llm::EMBED_MODEL_ID`). */
const EMBED_MODEL_ID = "emb-bge-m3-q8";

interface DownloadProgress {
  model_id: string;
  percentage: number;
}

/**
 * Semantische Suche (M4-P4b, E6): Schalter, Download des Suchmodells nur per
 * Knopf und der Indexstand "412 / 500 Besprechungen". Zeilen der bestehenden
 * Gruppe "Besprechungen", kein eigener Reiter.
 */
export const MeetingSemanticSearchSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const result = await commands.meetingIndexStatus();
    if (result.status === "ok") setStatus(result.data);
  }, []);

  useEffect(() => {
    void refresh();
    const unIndex = events.meetingIndexEvent.listen(() => void refresh());
    const unProgress = listen<DownloadProgress>(
      "model-download-progress",
      (e) => {
        if (e.payload.model_id === EMBED_MODEL_ID) {
          setPercent(Math.round(e.payload.percentage));
        }
      },
    );
    const unDone = listen<string>("model-download-complete", (e) => {
      if (e.payload === EMBED_MODEL_ID) {
        setPercent(null);
        void refresh();
      }
    });
    return () => {
      void unIndex.then((f) => f());
      void unProgress.then((f) => f());
      void unDone.then((f) => f());
    };
  }, [refresh]);

  const enabled = getSetting("meeting_semantic_search") ?? true;

  const download = async () => {
    setError(null);
    setPercent(0);
    const result = await commands.meetingEmbeddingModelDownload();
    setPercent(null);
    if (result.status === "error") setError(String(result.error));
    await refresh();
  };

  const downloading = percent !== null || (status?.downloading ?? false);

  let state: React.ReactNode;
  if (!status) {
    state = null;
  } else if (!status.model_ready) {
    state = (
      <Button
        variant="secondary"
        size="sm"
        onClick={() => void download()}
        disabled={downloading}
        data-testid="semantic-search-download"
      >
        {downloading
          ? t("settings.meetings.semanticSearchDownloading", {
              percent: percent ?? 0,
            })
          : t("settings.meetings.semanticSearchDownload")}
      </Button>
    );
  } else if (!enabled) {
    state = (
      <span className="text-sm text-mid-gray">
        {t("settings.meetings.semanticSearchOff")}
      </span>
    );
  } else {
    state = (
      <span
        className="text-sm text-mid-gray"
        data-testid="semantic-search-progress"
      >
        {t("settings.meetings.semanticSearchProgress", {
          done: status.embedded,
          total: status.total,
        })}
        {status.last_error
          ? ` · ${t("settings.meetings.semanticSearchPaused")}`
          : ""}
      </span>
    );
  }

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => void updateSetting("meeting_semantic_search", v)}
        isUpdating={isUpdating("meeting_semantic_search")}
        label={t("settings.meetings.semanticSearch")}
        description={t("settings.meetings.semanticSearchDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />
      <SettingContainer
        title={t("settings.meetings.semanticSearchIndex")}
        description={t("settings.meetings.semanticSearchIndexDescription")}
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          {state}
          {error && (
            <span className="text-xs text-red-400">
              {t("settings.meetings.semanticSearchDownloadFailed")}
            </span>
          )}
        </div>
      </SettingContainer>
    </>
  );
};
