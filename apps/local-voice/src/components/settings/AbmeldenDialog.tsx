import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Radio, RadioGroup } from "@fluentui/react-components";
import { commands, type SyncStatus } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";

interface Props {
  open: boolean;
  konto: string;
  onOpenChange: (open: boolean) => void;
  onAbgemeldet: (status: SyncStatus, entfernt: boolean) => void;
  /** Nach einem Fehler: Stand neu laden (das Abmelden kann trotzdem erfolgt sein). */
  onFehler: () => void;
}

/**
 * Abmelden (Spec 2026-11-08, U7). Vorausgewählt ist „behalten“. „Entfernen“
 * sichert und prüft zuerst; erst danach verschwinden Bibliothek und Seiten
 * vom Gerät. Besprechungen, Diktate, Stimmen und Modelle bleiben immer.
 */
export const AbmeldenDialog: React.FC<Props> = ({
  open,
  konto,
  onOpenChange,
  onAbgemeldet,
  onFehler,
}) => {
  const { t } = useTranslation();
  const [wahl, setWahl] = useState<"behalten" | "entfernen">("behalten");
  const [laeuft, setLaeuft] = useState(false);
  const [fehler, setFehler] = useState("");

  useEffect(() => {
    if (open) {
      setWahl("behalten");
      setFehler("");
    }
  }, [open]);

  const abmelden = async () => {
    if (laeuft) return;
    setLaeuft(true);
    setFehler("");
    const entfernen = wahl === "entfernen";
    const r = await commands.syncLogout(entfernen);
    setLaeuft(false);
    if (r.status === "error") {
      setFehler(r.error);
      onFehler();
      return;
    }
    onAbgemeldet(r.data, entfernen);
    onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!laeuft) onOpenChange(o);
      }}
      dismissible={!laeuft}
      title={t("sync.abmelden.titel")}
      description={t("sync.abmelden.beschreibung", { konto })}
      closeLabel={t("sync.abmelden.abbrechen")}
      className="max-w-lg"
      footer={
        <>
          <Button
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            variant="primary"
            onClick={() => void abmelden()}
            disabled={laeuft}
            data-testid="abmelden-bestaetigen"
          >
            {laeuft
              ? wahl === "entfernen"
                ? t("sync.anmeldung.laeuft")
                : t("sync.abmelden.laeuft")
              : t("sync.abmelden.knopf")}
          </Button>
          <Button
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={laeuft}
          >
            {t("sync.abmelden.abbrechen")}
          </Button>
        </>
      }
    >
      <div className="space-y-3 text-sm" data-testid="abmelden-dialog">
        <RadioGroup
          value={wahl}
          onChange={(_, d) => setWahl(d.value as "behalten" | "entfernen")}
          aria-label={t("sync.abmelden.titel")}
        >
          <Radio
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            value="behalten"
            label={
              <span>
                <span className="font-medium">
                  {t("sync.abmelden.behalten")}
                </span>
                <span className="block text-text-muted">
                  {t("sync.abmelden.behaltenText")}
                </span>
              </span>
            }
            data-testid="abmelden-behalten"
          />
          <Radio
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            value="entfernen"
            label={
              <span>
                <span className="font-medium">
                  {t("sync.abmelden.entfernen")}
                </span>
                <span className="block text-text-muted">
                  {t("sync.abmelden.entfernenText")}
                </span>
              </span>
            }
            data-testid="abmelden-entfernen"
          />
        </RadioGroup>
        <p className="text-text-muted">{t("sync.anmeldung.unberuehrt")}</p>
        {fehler && (
          <p className="text-red-500" role="alert">
            {fehler}
          </p>
        )}
      </div>
    </Dialog>
  );
};
