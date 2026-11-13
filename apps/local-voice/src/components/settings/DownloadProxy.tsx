import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { useSettings } from "../../hooks/useSettings";
import { commands } from "@/bindings";

interface DownloadProxyProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * lva#573: Proxy für Modell-Downloads. Leer = wie Windows (PAC-Datei,
 * Auto-Erkennung, Systemproxy); „direkt“ erzwingt ohne Proxy. Die Prüfung
 * der Eingabe macht das Backend; seine Meldung steht unter dem Feld.
 */
export const DownloadProxy: React.FC<DownloadProxyProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, refreshSettings } = useSettings();
    const gespeichert = getSetting("download_proxy") ?? "";
    const [entwurf, setEntwurf] = useState(gespeichert);
    const [fehler, setFehler] = useState<string | null>(null);
    const [speichert, setSpeichert] = useState(false);

    useEffect(() => setEntwurf(gespeichert), [gespeichert]);

    const speichern = async (wert: string) => {
      setSpeichert(true);
      setFehler(null);
      try {
        const r = await commands.changeDownloadProxySetting(
          wert.trim() ? wert.trim() : null,
        );
        if (r.status === "error") {
          setFehler(r.error);
        } else {
          await refreshSettings();
        }
      } finally {
        setSpeichert(false);
      }
    };

    return (
      <SettingContainer
        title={t("settings.app.downloadProxy.title")}
        description={t("settings.app.downloadProxy.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="flex flex-col gap-1 min-w-0">
          <div className="flex items-center gap-2 min-w-0">
            <Input
              variant="compact"
              value={entwurf}
              placeholder={t("settings.app.downloadProxy.placeholder")}
              aria-label={t("settings.app.downloadProxy.title")}
              onChange={(e) => setEntwurf(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void speichern(entwurf);
              }}
              className="w-[220px]"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void speichern(entwurf)}
              disabled={speichert || entwurf.trim() === gespeichert}
            >
              {t("settings.app.downloadProxy.save")}
            </Button>
            {gespeichert && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void speichern("")}
                disabled={speichert}
              >
                {t("settings.app.downloadProxy.reset")}
              </Button>
            )}
          </div>
          {fehler && (
            <span role="alert" className="text-xs text-red-500">
              {fehler}
            </span>
          )}
        </div>
      </SettingContainer>
    );
  },
);

DownloadProxy.displayName = "DownloadProxy";
