import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";
import type { DictationAudio as DictationAudioMode } from "@/bindings";

interface DictationAudioProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * What happens to music or video output while a dictation runs: nothing,
 * mute, lower to a level, or pause the player. Default is "lower to 10 %",
 * so playback neither disturbs the speaker nor ends up in the recording.
 */
export const DictationAudio: React.FC<DictationAudioProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const mode = (getSetting("dictation_audio") ?? "duck") as DictationAudioMode;
    const percent = getSetting("dictation_audio_duck_percent") ?? 10;

    const options = (["off", "mute", "duck", "pause"] as DictationAudioMode[]).map(
      (value) => ({
        value,
        label: t(`settings.sound.dictationAudio.options.${value}`),
      }),
    );

    return (
      <>
        <SettingContainer
          title={t("settings.sound.dictationAudio.title")}
          description={t("settings.sound.dictationAudio.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        >
          <Dropdown
            options={options}
            selectedValue={mode}
            onSelect={(value) =>
              updateSetting("dictation_audio", value as DictationAudioMode)
            }
            disabled={isUpdating("dictation_audio")}
          />
        </SettingContainer>
        {mode === "duck" && (
          <Slider
            value={percent}
            onChange={(value: number) =>
              updateSetting("dictation_audio_duck_percent", Math.round(value))
            }
            min={0}
            max={50}
            step={5}
            label={t("settings.sound.dictationAudio.duck.title")}
            description={t("settings.sound.dictationAudio.duck.description")}
            descriptionMode={descriptionMode}
            grouped={grouped}
            formatValue={(value) => `${Math.round(value)}%`}
            disabled={isUpdating("dictation_audio_duck_percent")}
          />
        )}
      </>
    );
  },
);
