import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Button } from "../../ui/Button";

const NVIDIA_OPEN_MODEL_LICENSE_URL =
  "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/";

export const AboutSettings: React.FC = () => {
  const { t } = useTranslation();
  const [version, setVersion] = useState("");

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("0.1.2");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="w-full space-y-6">
      <SettingsGroup title={t("settings.about.title")}>
        <SettingContainer
          title={t("settings.about.version.title")}
          description={t("settings.about.version.description")}
          grouped={true}
        >
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span className="text-sm font-mono">v{version}</span>
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.sourceCode.title")}
          description={t("settings.about.sourceCode.description")}
          grouped={true}
        >
          <Button
            variant="secondary"
            size="md"
            onClick={() => openUrl("https://github.com/MrP42/local-voice-ai")}
          >
            {t("settings.about.sourceCode.button")}
          </Button>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup title={t("settings.about.acknowledgments.title")}>
        <SettingContainer
          title={t("settings.about.acknowledgments.ggml.title")}
          description={t("settings.about.acknowledgments.ggml.description")}
          grouped={true}
          layout="stacked"
        >
          <div className="text-sm text-mid-gray">
            {t("settings.about.acknowledgments.ggml.details")}
          </div>
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.acknowledgments.models.title")}
          description={t("settings.about.acknowledgments.models.description")}
          grouped={true}
          layout="stacked"
        >
          <div className="text-sm text-mid-gray">
            {t("settings.about.acknowledgments.models.details")}
          </div>
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.acknowledgments.sortformer.title")}
          description={t(
            "settings.about.acknowledgments.sortformer.description",
          )}
          grouped={true}
          layout="stacked"
        >
          <div className="space-y-2">
            <div className="text-sm text-mid-gray">
              {t("settings.about.acknowledgments.sortformer.details")}
            </div>
            <Button
              variant="secondary"
              size="md"
              onClick={() => openUrl(NVIDIA_OPEN_MODEL_LICENSE_URL)}
            >
              {t("settings.about.acknowledgments.sortformer.button")}
            </Button>
          </div>
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.acknowledgments.libraries.title")}
          description={t(
            "settings.about.acknowledgments.libraries.description",
          )}
          grouped={true}
          layout="stacked"
        >
          <div className="text-sm text-mid-gray">
            {t("settings.about.acknowledgments.libraries.details")}
          </div>
        </SettingContainer>
      </SettingsGroup>
    </div>
  );
};
