import React from "react";
import { useTranslation } from "react-i18next";
import { PageShell } from "../ui/PageShell";
import { openSettingsTab } from "../../lib/openSettingsTab";
import { Profilseite } from "./Profilseite";
import { useKonto } from "./useKonto";
import { useKontoLabels } from "./labels";

/**
 * Ansicht „Mein Profil“ (ADR-0024 E6). Kein Eintrag in der Seitenleiste: sie öffnet über
 * das Profilmenü oben rechts. Den Titel setzt die Kopfleiste (PageShell), die Seite hat kein eigenes h1.
 */
export const KontoProfilSeite: React.FC = () => {
  const { t } = useTranslation();
  const konto = useKonto();
  const labels = useKontoLabels();
  return (
    <PageShell
      title={t("konto.meinProfil")}
      description={t("konto.beschreibung")}
    >
      {/* Die Beschreibung steht schon in der Kopfleiste. */}
      <div className="[&_.wai-profilseite__beschreibung]:hidden">
        <Profilseite
          profil={konto.aktiv}
          status={konto.status}
          onSpeichern={konto.aendern}
          onBild={konto.bildSetzen}
          onBildEntfernen={() => void konto.bildEntfernen()}
          onKontoGeraete={() => openSettingsTab("account")}
          labels={labels}
        />
      </div>
    </PageShell>
  );
};
