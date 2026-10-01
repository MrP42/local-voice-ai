import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { commands, type ToolStatus } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { SettingContainer } from "../../../ui/SettingContainer";
import { ToggleSwitch } from "../../../ui/ToggleSwitch";
import { useSettings } from "../../../../hooks/useSettings";

/**
 * YouTube als Zeilen der Gruppe "Besprechungen" (#65, E1): der Schalter
 * "privat/experimentell" (Standard aus) und, wenn er an ist, der Pfad eines
 * SELBST installierten yt-dlp mit Erkennung ("gefunden: Version ...") und einem
 * kurzen Hinweis zur Rechtslage. Die App buendelt yt-dlp nicht und laedt es nicht
 * herunter; in diesem Schritt wird es nur gefunden, nicht benutzt.
 */
export const MeetingYoutubeSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const enabled = getSetting("meeting_youtube_private") ?? false;
  const saved = getSetting("meeting_youtube_tool_path") ?? "";
  const [path, setPath] = useState(saved);
  const [status, setStatus] = useState<ToolStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const latest = useRef(0);

  // Der gespeicherte Wert kommt erst nach dem Laden der Einstellungen.
  useEffect(() => {
    setPath(saved);
  }, [saved]);

  const detect = useCallback(async (value: string) => {
    const run = ++latest.current;
    setChecking(true);
    try {
      const result = await commands.youtubeToolDetect(value);
      if (run === latest.current && result.status === "ok") {
        setStatus(result.data);
      }
    } finally {
      if (run === latest.current) setChecking(false);
    }
  }, []);

  // Nur solange der Schalter an ist wird nach dem Programm gesucht.
  useEffect(() => {
    if (enabled) void detect(saved);
    else setStatus(null);
  }, [enabled, saved, detect]);

  const commitPath = (value: string) => {
    const trimmed = value.trim();
    if (trimmed === saved) {
      void detect(trimmed);
      return;
    }
    void updateSetting("meeting_youtube_tool_path", trimmed || null);
  };

  const browse = async () => {
    const picked = await open({ multiple: false });
    if (typeof picked === "string") {
      setPath(picked);
      commitPath(picked);
    }
  };

  const statusLine = () => {
    if (checking && !status)
      return t("meetings.youtube.settings.tool.checking");
    if (!status) return null;
    if (status.found) {
      return t("meetings.youtube.settings.tool.found", {
        version: status.version ?? "?",
        path: status.path ?? "",
      });
    }
    return t(
      `meetings.youtube.settings.tool.errors.${status.error ?? "not_found"}`,
      {
        defaultValue: t("meetings.youtube.settings.tool.errors.not_found"),
      },
    );
  };

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => void updateSetting("meeting_youtube_private", v)}
        isUpdating={isUpdating("meeting_youtube_private")}
        label={t("meetings.youtube.settings.private")}
        description={t("meetings.youtube.settings.privateDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />
      {enabled && (
        <SettingContainer
          title={t("meetings.youtube.settings.tool.title")}
          description={t("meetings.youtube.settings.tool.description")}
          grouped={true}
          layout="stacked"
          descriptionMode="tooltip"
        >
          <div className="space-y-2" data-testid="yt-tool-settings">
            <div className="flex flex-wrap items-center gap-2">
              <Input
                type="text"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                onBlur={() => commitPath(path)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitPath(path);
                }}
                placeholder={t("meetings.youtube.settings.tool.placeholder")}
                aria-label={t("meetings.youtube.settings.tool.title")}
                className="min-w-0 flex-1"
                spellCheck={false}
                data-testid="yt-tool-path"
              />
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void browse()}
                data-testid="yt-tool-browse"
              >
                {t("meetings.youtube.settings.tool.browse")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onClick={() => commitPath(path)}
                disabled={checking}
                data-testid="yt-tool-check"
              >
                {t("meetings.youtube.settings.tool.check")}
              </Button>
            </div>
            <p
              className={`break-words text-sm ${
                status?.found ? "text-green-500" : "text-text/70"
              }`}
              role="status"
              data-testid="yt-tool-status"
              data-found={status?.found ? "true" : "false"}
            >
              {statusLine()}
            </p>
            <p
              className="text-xs text-amber-600 dark:text-amber-400"
              role="note"
              data-testid="yt-tool-legal"
            >
              {t("meetings.youtube.settings.legal")}
            </p>
          </div>
        </SettingContainer>
      )}
    </>
  );
};
