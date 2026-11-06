import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
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
  // Die Ziel-ID ist das Profil, das diese Ansicht gerade zeigt; der Kern lehnt ab, wenn
  // inzwischen ein anderes aktiv ist.
  const id = konto.aktiv.id;
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
          onSpeichern={(aenderung) => konto.aendern(id, aenderung)}
          onBild={(datei) => konto.bildSetzen(id, datei)}
          onBildEntfernen={() =>
            void konto
              .bildEntfernen(id)
              .catch(() => toast.error(t("konto.bildEntfernenFehler")))
          }
          onKontoGeraete={() => openSettingsTab("account")}
          labels={labels}
        />
      </div>
    </PageShell>
  );
};
