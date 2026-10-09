import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { AudioFeedback } from "../AudioFeedback";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { VolumeSlider } from "../VolumeSlider";
import { useSettings } from "../../../hooks/useSettings";

/** Der hoerbare Teil der Ausgabe: Signaltoene, Ausgabegeraet, Lautstaerke. */
export const SoundTab: React.FC = () => {
  const { t } = useTranslation();
  const { audioFeedbackEnabled } = useSettings();

  return (
    <div className="w-full space-y-6">
      <SettingsGroup title={t("settings.app.groups.feedback")}>
        <AudioFeedback descriptionMode="tooltip" grouped={true} />
        <OutputDeviceSelector
          descriptionMode="tooltip"
          grouped={true}
          disabled={!audioFeedbackEnabled}
        />
        <VolumeSlider disabled={!audioFeedbackEnabled} />
      </SettingsGroup>
    </div>
  );
};
