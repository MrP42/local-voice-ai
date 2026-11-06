import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";
import { commands } from "../../bindings";

interface LocalUpdateAutoInstallProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Entwicklungskanal: signierte Testversionen aus dem lokalen Update-Ordner
 * selbst einspielen (App schließt sich, installiert, startet neu). Nur in
 * Abnahme-Bauten mit Entwicklungsschlüssel sichtbar; Releases kennen den
 * Kanal nicht. Ohne eigene Wahl an, ein gespeichertes „aus“ bleibt aus.
 */
export const LocalUpdateAutoInstall: React.FC<LocalUpdateAutoInstallProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [vorhanden, setVorhanden] = useState(false);

    useEffect(() => {
      void commands
        .devUpdateAvailable()
        .then(setVorhanden)
        .catch(() => setVorhanden(false));
    }, []);

    if (!vorhanden) return null;
    const an = getSetting("local_update_auto_install") ?? true;

    return (
      <ToggleSwitch
        checked={an}
        onChange={(enabled) =>
          updateSetting("local_update_auto_install", enabled)
        }
        isUpdating={isUpdating("local_update_auto_install")}
        label={t("settings.app.localUpdateAuto.label")}
        description={t("settings.app.localUpdateAuto.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        tooltipPosition="bottom"
      />
    );
  });
