import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import type { DeviceGroup, LicenseKind, VoiceItem } from "./model";

/** Beschriftungen der Stimmenverwaltung: Sprache, Geraet, Lizenz. */
export function useVoiceLabels() {
  const { t, i18n } = useTranslation();
  const language = useCallback(
    (code: string) => {
      if (code === "und") return t("settings.models.voiceManager.multilingual");
      try {
        return (
          new Intl.DisplayNames([i18n.language], { type: "language" }).of(
            code,
          ) ?? code
        );
      } catch {
        return code;
      }
    },
    [t, i18n.language],
  );
  const device = useCallback(
    (d: DeviceGroup) => t(`settings.models.voiceManager.device.${d}`),
    [t],
  );
  const license = useCallback(
    (k: LicenseKind) => t(`settings.models.voiceManager.license.${k}`),
    [t],
  );
  const gender = useCallback(
    (g: VoiceItem["gender"]) =>
      g ? t(`tts.providers.voices.gender.${g}`, { defaultValue: g }) : null,
    [t],
  );
  return { language, device, license, gender };
}
