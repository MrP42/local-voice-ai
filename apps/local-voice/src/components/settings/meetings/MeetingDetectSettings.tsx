import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type DetectMode } from "@/bindings";
import { Dropdown } from "../../ui/Dropdown";
import { Input } from "../../ui/Input";
import { SettingContainer } from "../../ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";

/** Kommagetrennte Eingabe in eine saubere Liste (ohne Leeres, ohne Doppelte). */
export const parseIgnoredApps = (text: string): string[] => {
  const seen = new Set<string>();
  return text
    .split(/[,;\n]/)
    .map((part) => part.trim())
    .filter((part) => {
      const key = part.toLowerCase();
      if (!part || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
};

/**
 * Ad-hoc-Erkennung laufender Besprechungen (M5-P5c, F16): Modus und
 * Ignorierliste als Zeilen der bestehenden Gruppe "Besprechungen", kein
 * eigener Reiter. Wirkt ohne Neustart (der Watcher liest die Einstellung
 * alle 2 s).
 */
export const MeetingDetectSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [available, setAvailable] = useState(true);
  const stored = (getSetting("meeting_detect_ignored_apps") ?? []).join(", ");
  const [draft, setDraft] = useState(stored);

  useEffect(() => {
    let cancelled = false;
    void commands
      .meetingDetectAvailable()
      .then((ok) => {
        if (!cancelled) setAvailable(ok !== false);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    setDraft(stored);
  }, [stored]);

  const mode: DetectMode = getSetting("meeting_detect_mode") ?? "meeting_apps";
  const options: { value: DetectMode; label: string }[] = [
    { value: "off", label: t("settings.meetings.detectOff") },
    { value: "meeting_apps", label: t("settings.meetings.detectMeetingApps") },
    { value: "all_apps", label: t("settings.meetings.detectAllApps") },
  ];

  const commit = () => {
    const list = parseIgnoredApps(draft);
    if (list.join(", ") !== stored) {
      void updateSetting("meeting_detect_ignored_apps", list);
    }
  };

  return (
    <>
      <SettingContainer
        title={t("settings.meetings.detect")}
        description={
          available
            ? t("settings.meetings.detectDescription")
            : t("settings.meetings.detectUnavailable")
        }
        grouped={true}
      >
        <Dropdown
          options={options}
          selectedValue={available ? mode : "off"}
          onSelect={(value) =>
            void updateSetting("meeting_detect_mode", value as DetectMode)
          }
          disabled={!available || isUpdating("meeting_detect_mode")}
        />
      </SettingContainer>
      {available && mode !== "off" && (
        <SettingContainer
          title={t("settings.meetings.detectIgnored")}
          description={t("settings.meetings.detectIgnoredDescription")}
          grouped={true}
        >
          <Input
            variant="compact"
            className="w-56"
            value={draft}
            placeholder={t("settings.meetings.detectIgnoredPlaceholder")}
            disabled={isUpdating("meeting_detect_ignored_apps")}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
          />
        </SettingContainer>
      )}
    </>
  );
};
