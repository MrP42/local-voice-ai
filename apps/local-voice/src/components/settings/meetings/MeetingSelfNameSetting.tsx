import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "../../ui/Input";
import { SettingContainer } from "../../ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";

/**
 * U8: "Mein Name". Das eigene Mikrofon (Kanal "Ich") und bei getrenntem
 * Mikrofon der Sprecher mit dem größten Anteil tragen immer diesen Namen,
 * auch wenn niemand ihn anspricht; in Personen ist er als "ich" markiert.
 * Gespeichert wird beim Verlassen des Feldes oder mit Enter; leer = "Ich".
 */
export const MeetingSelfNameSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const stored = (getSetting("meeting_self_name") ?? "").trim();
  const [draft, setDraft] = useState(stored);

  // Ein anderer Stand (Sync, zweites Fenster) überschreibt den Entwurf.
  useEffect(() => {
    setDraft(stored);
  }, [stored]);

  const commit = () => {
    const value = draft.trim();
    if (value === stored) return;
    void updateSetting("meeting_self_name", value === "" ? null : value);
  };

  return (
    <SettingContainer
      title={t("settings.meetings.selfName")}
      description={t("settings.meetings.selfNameDescription")}
      grouped={true}
    >
      <Input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
        placeholder={t("settings.meetings.selfNamePlaceholder")}
        aria-label={t("settings.meetings.selfName")}
        maxLength={80}
        autoComplete="off"
        disabled={isUpdating("meeting_self_name")}
        data-testid="self-name-input"
        className="w-56"
      />
    </SettingContainer>
  );
};
