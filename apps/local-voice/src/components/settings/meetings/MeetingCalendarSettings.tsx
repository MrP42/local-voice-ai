import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";
import { IntegrationsLinkRow } from "./IntegrationsLinkRow";

const REMINDER_CHOICES = [
  { value: "0", key: "off" },
  { value: "60", key: "minute1" },
  { value: "120", key: "minutes2" },
  { value: "300", key: "minutes5" },
] as const;

/**
 * Kalender und Erinnerung als Zeilen der Gruppe "Besprechungen". Die Quellen
 * (verbinden, Stand, entfernen) wohnen seit A4 auf der Seite "Integrationen";
 * hier bleiben der Verweis dorthin und die Erinnerung vor Terminen, die zu den
 * Besprechungen gehoert und keine Verbindung ist.
 */
export const MeetingCalendarSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const lead = String(getSetting("meeting_reminder_lead_s") ?? 60);
  const reminderOptions: { value: string; label: string }[] =
    REMINDER_CHOICES.map((c) => ({
      value: c.value,
      label: t(`meetings.calendar.reminder.${c.key}`),
    }));
  // Ein eigener Wert aus der Einstellungsdatei (z. B. 90 s) bleibt sichtbar.
  if (!reminderOptions.some((o) => o.value === lead)) {
    reminderOptions.push({
      value: lead,
      label: `${Math.round(Number(lead) / 60)} min`,
    });
  }

  return (
    <>
      <IntegrationsLinkRow topic="calendar" />
      <div data-testid="calendar-reminder-settings">
        <SettingContainer
          title={t("meetings.calendar.reminder.title")}
          description={t("meetings.calendar.reminder.description")}
          grouped={true}
        >
          <Dropdown
            options={reminderOptions}
            selectedValue={lead}
            onSelect={(value) =>
              void updateSetting("meeting_reminder_lead_s", Number(value))
            }
            disabled={isUpdating("meeting_reminder_lead_s")}
          />
        </SettingContainer>
        <ToggleSwitch
          checked={getSetting("meeting_reminder_all_events") ?? false}
          onChange={(v) => void updateSetting("meeting_reminder_all_events", v)}
          isUpdating={isUpdating("meeting_reminder_all_events")}
          disabled={lead === "0"}
          label={t("meetings.calendar.reminder.allEvents")}
          description={t("meetings.calendar.reminder.allEventsDescription")}
          descriptionMode="tooltip"
          grouped={true}
        />
      </div>
    </>
  );
};
