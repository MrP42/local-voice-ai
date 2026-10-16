import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { ComplianceProfile } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { useComplianceStore } from "@/stores/complianceStore";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Select } from "../../ui/Select";

const PROFILES: ComplianceProfile[] = ["eu", "local_only", "none"];

/**
 * Welche Regeln gelten für Sprachmodelle? Unter "EU" sind nur Anbieter
 * wählbar, deren dokumentierte Angaben EU-KI-VO und DSGVO genügen; gesperrte
 * Modelle bleiben sichtbar, bekommen aber keine Anfrage. Das Schild in der
 * Fußleiste zeigt das Ergebnis.
 */
export const ComplianceSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const { setProfile, refresh } = useComplianceStore();
  const profile = (getSetting("compliance_profile") ??
    "eu") as ComplianceProfile;

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
      <p className="px-4 pb-3 text-xs text-text/50">
        {t("settings.compliance.disclaimer")}
      </p>
    </SettingsGroup>
  );
};

export default ComplianceSettings;
