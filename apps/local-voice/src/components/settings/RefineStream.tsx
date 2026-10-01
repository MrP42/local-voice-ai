import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface RefineStreamProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Optionale Glaettung des live eingefuegten Textes durch ein lokales Ollama
 * (Issue #5). Standard aus. Ohne Live-Einfuegung gaebe es nichts zu glaetten:
 * dann zeigt die Einstellungsseite diesen Schalter gar nicht erst an.
 */
export const RefineStream: React.FC<RefineStreamProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    return (
      <ToggleSwitch
        checked={getSetting("refine_enabled") ?? false}
        onChange={(enabled) => updateSetting("refine_enabled", enabled)}
        isUpdating={isUpdating("refine_enabled")}
        label={t("settings.advanced.refineStream.label")}
        description={t("settings.advanced.refineStream.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
