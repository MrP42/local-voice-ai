import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type TemplateInfo } from "@/bindings";
import { DEFAULT_TEMPLATE_ID } from "@/lib/meetingNotes";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/**
 * Die drei Vorgaben der Aufnahme (M1-P1f), als Zeilen der bestehenden Gruppe
 * "Besprechungen": Systemton, automatische KI-Notizen, Standardvorlage.
 */
export const MeetingNotesSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    void commands.meetingTemplatesList().then((result) => {
      if (!cancelled && result.status === "ok") setTemplates(result.data);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const options = templates.map((info) => ({
    value: info.id,
    label: info.title,
  }));
  const stored = getSetting("meeting_default_template_id") ?? null;
  // Eine geloeschte oder unbekannte Vorlage zeigt die Standardvorlage
  // (wie der Motor, der bei unbekannter ID darauf zurueckfaellt).
  const selected =
    stored && templates.some((info) => info.id === stored)
      ? stored
      : DEFAULT_TEMPLATE_ID;

  return (
    <>
      <ToggleSwitch
        checked={getSetting("meeting_capture_system") ?? true}
        onChange={(v) => void updateSetting("meeting_capture_system", v)}
        isUpdating={isUpdating("meeting_capture_system")}
        label={t("settings.meetings.captureSystem")}
        description={t("settings.meetings.captureSystemDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />
      <ToggleSwitch
        checked={getSetting("meeting_auto_enhance") ?? true}
        onChange={(v) => void updateSetting("meeting_auto_enhance", v)}
        isUpdating={isUpdating("meeting_auto_enhance")}
        label={t("settings.meetings.autoEnhance")}
        description={t("settings.meetings.autoEnhanceDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />
      <SettingContainer
        title={t("settings.meetings.defaultTemplate")}
        description={t("settings.meetings.defaultTemplateDescription")}
        grouped={true}
      >
        <Dropdown
          options={options}
          selectedValue={selected}
          onSelect={(id) =>
            void updateSetting(
              "meeting_default_template_id",
              id === DEFAULT_TEMPLATE_ID ? null : id,
            )
          }
          placeholder={t("meetings.templates.label")}
          disabled={isUpdating("meeting_default_template_id")}
        />
      </SettingContainer>
    </>
  );
};
