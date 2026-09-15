import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { MicLevelMeter } from "../MicLevelMeter";
import { MicSensitivity } from "../MicSensitivity";
import { DictationAudio } from "../DictationAudio";
import { AudioFeedback } from "../AudioFeedback";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { VolumeSlider } from "../VolumeSlider";
import { useSettings } from "../../../hooks/useSettings";

/** Input and output devices, plus the audible feedback the app gives back. */
export const SoundTab: React.FC = () => {
  const { t } = useTranslation();
  const { audioFeedbackEnabled } = useSettings();

  return (
    <div className="w-full space-y-6">
      <SettingsGroup title={t("settings.app.groups.input")}>
        <MicrophoneSelector descriptionMode="tooltip" grouped={true} />
        {/* Right below the picker: whether the device you just chose actually
            hears you is the first thing you want to know about it. */}
        <div className="px-3 pb-3">
          <MicLevelMeter compact />
        </div>
        <MicSensitivity />
        <DictationAudio descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

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
