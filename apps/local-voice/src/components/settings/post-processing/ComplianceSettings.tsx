import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { ComplianceProfile } from "@/bindings";
import { noticeSettingsOf } from "@/lib/meetingNotice";
import { useSettings } from "../../../hooks/useSettings";
import { useComplianceStore } from "@/stores/complianceStore";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Select } from "../../ui/Select";
import { ToggleSwitch } from "../../ui/ToggleSwitch";

const PROFILES: ComplianceProfile[] = ["eu", "local_only", "none"];

/**
 * Welche Regeln gelten für Sprachmodelle? Unter "EU" sind nur Anbieter
 * wählbar, deren dokumentierte Angaben EU-KI-VO und DSGVO genügen; gesperrte
 * Modelle bleiben sichtbar, bekommen aber keine Anfrage. Das Schild in der
 * Fußleiste zeigt das Ergebnis.
 */
export const ComplianceSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings, updateSetting, isUpdating } =
    useSettings();
  const { setProfile, refresh } = useComplianceStore();
  const profile = (getSetting("compliance_profile") ??
    "eu") as ComplianceProfile;

  // G2 (#497): Pflicht zum Hinweis an Teilnehmende vor Aufnahmebeginn.
  const notice = noticeSettingsOf(getSetting("meeting_notice"));

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <SettingsGroup title={t("settings.compliance.title")}>
      <SettingContainer
        title={t("settings.compliance.profile")}
        description={t(`settings.compliance.profiles.${profile}.description`)}
        descriptionMode="inline"
        layout="horizontal"
        grouped={true}
      >
        <div className="w-72" data-compliance-profile={profile}>
          <Select
            value={profile}
            options={PROFILES.map((p) => ({
              value: p,
              label: t(`settings.compliance.profiles.${p}.label`),
            }))}
            isClearable={false}
            onChange={(value) => {
              if (!value) return;
              void setProfile(value as ComplianceProfile).then(() =>
                refreshSettings(),
              );
            }}
          />
        </div>
      </SettingContainer>
      <ToggleSwitch
        checked={notice.required}
        onChange={(v) =>
          void updateSetting("meeting_notice", { ...notice, required: v })
        }
        isUpdating={isUpdating("meeting_notice")}
        disabled={!notice.enabled}
        label={t("settings.compliance.noticeRequired")}
        description={
          notice.enabled
            ? t("settings.compliance.noticeRequiredDescription")
            : t("settings.compliance.noticeRequiredNeedsFeature")
        }
        descriptionMode="inline"
        grouped={true}
        testId="notice-required"
      />
      <p className="px-4 pb-3 text-xs text-text/50">
        {t("settings.compliance.disclaimer")}
      </p>
    </SettingsGroup>
  );
};

export default ComplianceSettings;
