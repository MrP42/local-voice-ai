import React from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { SettingContainer } from "../ui/SettingContainer";
import { Button } from "../ui/Button";
import { useSettings } from "../../hooks/useSettings";

interface LocalUpdateDirectoryProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Folder the app scans for newer installers (local acceptance builds that
 * never reach GitHub). Empty means off; the footer's update check then only
 * asks GitHub.
 */
export const LocalUpdateDirectory: React.FC<LocalUpdateDirectoryProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const dir = getSetting("local_update_dir") ?? null;

    const choose = async () => {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) {
        updateSetting("local_update_dir", picked);
      }
    };

    return (
      <SettingContainer
        title={t("settings.app.localUpdateDir.title")}
        description={t("settings.app.localUpdateDir.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="flex items-center gap-2 min-w-0">
          <span
            className="text-sm text-text/70 truncate max-w-[260px]"
            title={dir ?? undefined}
          >
            {dir ?? t("settings.app.localUpdateDir.none")}
          </span>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void choose()}
            disabled={isUpdating("local_update_dir")}
          >
            {t("settings.app.localUpdateDir.choose")}
          </Button>
          {dir && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => updateSetting("local_update_dir", null)}
              disabled={isUpdating("local_update_dir")}
            >
              {t("settings.app.localUpdateDir.clear")}
            </Button>
          )}
        </div>
      </SettingContainer>
    );
  });
