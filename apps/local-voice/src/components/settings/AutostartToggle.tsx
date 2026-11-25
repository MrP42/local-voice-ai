import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";
import { useFirmenrichtlinie } from "../../hooks/useFirmenrichtlinie";

interface AutostartToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const AutostartToggle: React.FC<AutostartToggleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const gesperrt = useFirmenrichtlinie().gesperrt("kein_autostart");

    const autostartEnabled =
      !gesperrt && (getSetting("autostart_enabled") ?? false);

    return (
      <ToggleSwitch
        checked={autostartEnabled}
        onChange={(enabled) => updateSetting("autostart_enabled", enabled)}
        isUpdating={isUpdating("autostart_enabled")}
        label={t("settings.advanced.autostart.label")}
        description={
          gesperrt
            ? t("firmenrichtlinie.gesperrt.kein_autostart")
            : t("settings.advanced.autostart.description")
        }
        descriptionMode={gesperrt ? "inline" : descriptionMode}
        disabled={gesperrt}
        grouped={grouped}
      />
    );
  },
);
