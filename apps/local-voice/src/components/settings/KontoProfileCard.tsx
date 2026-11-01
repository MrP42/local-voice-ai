import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Plus } from "lucide-react";
import { SettingsGroup } from "../ui/SettingsGroup";
import { Button } from "../ui/Button";
import Badge from "../ui/Badge";
import { ProfilBild } from "../konto/Profilknopf";
import { ProfilHinzufuegenDialog } from "../konto/ProfilHinzufuegenDialog";
import { profilName } from "../konto/profil";
import { useKontoLabels } from "../konto/labels";
import { useKonto } from "../konto/useKonto";

/**
 * „Profile auf diesem PC“ (ADR-0024 E4, Paket 1: nur lokale Profile). Zeigt die Profile des
 * gemeinsamen Kontospeichers, kennzeichnet das aktive und legt neue an.
 */
export const KontoProfileCard: React.FC = () => {
  const { t } = useTranslation();
  const konto = useKonto();
  const labels = useKontoLabels();
  const [dialog, setDialog] = useState(false);
  const alle = [konto.aktiv, ...konto.weitere];

  return (
    <SettingsGroup
      title={t("konto.profileKarte.titel")}
      description={t("konto.profileKarte.beschreibung")}
      anchor="konto-profile"
    >
      <ul
        className="divide-y divide-mid-gray/20"
        data-testid="konto-profilliste"
      >
        {alle.map((p) => {
          const aktiv = p.id === konto.aktiv.id;
          const name = profilName(p, labels.lokalesProfil);
          return (
            <li
              key={p.id || "ersatz"}
              className="flex items-center gap-3 px-4 py-3"
              data-aktiv={aktiv || undefined}
            >
              <ProfilBild profil={p} groesse={32} labels={labels} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">{name}</div>
                <div className="truncate text-xs text-text-muted">
                  {p.email ?? labels.status.lokal}
                </div>
              </div>
              {aktiv ? (
                <Badge variant="primary">{t("konto.profileKarte.aktiv")}</Badge>
              ) : (
                <Button
                  variant="secondary"
                  size="sm"
                  aria-label={labels.profilWechseln(name)}
                  onClick={() =>
                    konto
                      .aktivieren(p.id)
                      .catch(() => toast.error(t("konto.wechselnFehler")))
                  }
                >
                  {t("konto.profileKarte.wechseln")}
                </Button>
              )}
            </li>
          );
        })}
      </ul>
      <div className="px-4 py-3">
        <Button
          variant="secondary"
          size="sm"
          icon={<Plus size={14} aria-hidden="true" />}
          onClick={() => setDialog(true)}
          disabled={!konto.verfuegbar}
          data-testid="konto-profil-hinzufuegen"
        >
          {labels.profilHinzufuegen}
        </Button>
      </div>
      <ProfilHinzufuegenDialog
        open={dialog}
        onOpenChange={setDialog}
        onAnlegen={konto.anlegen}
      />
    </SettingsGroup>
  );
};
