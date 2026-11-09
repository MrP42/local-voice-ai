import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Switch } from "@fluentui/react-components";
import { listen } from "@tauri-apps/api/event";
import { Sparkles } from "lucide-react";
import type { AblageSemantik } from "@/bindings";
import { Button } from "../ui/Button";
import { api } from "./api";

/** Kennung des Modells in `model-download-progress` (siehe `ablage::semantik::MODELL_ID`). */
const MODELL_ID = "ablage-e5-small";

/** Statuszeile der Einstellung als Text (ohne Prozent). */
function statusSchluessel(s: AblageSemantik): string {
  if (!s.an) return "aus";
  if (s.laedt) return "laedt";
  if (!s.modell_bereit) return "fehlt";
  if (s.ladefehler) return "fehler";
  if (s.nachziehen) return "nachziehen";
  if (!s.aktiv) return "wartet";
  return "aktiv";
}

/**
 * D6: Einstellung „Semantische Suche“ in den Suchergebnissen der Ablage, mit Modellstatus und
 * Download. Ohne Modell bleibt die Suche wörtlich (lexikalisch).
 */
export const SemantikZeile: React.FC<{ onGeaendert(): void }> = ({
  onGeaendert,
}) => {
  const { t } = useTranslation();
  const [s, setS] = useState<AblageSemantik | null>(null);
  const [prozent, setProzent] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const laden = useCallback(() => {
    api
      .semantikStand()
      .then(setS)
      .catch(() => setS(null));
  }, []);

  useEffect(() => {
    laden();
  }, [laden]);

  useEffect(() => {
    let aktiv = true;
    const aus: (() => void)[] = [];
    const merken = (f: () => void) => (aktiv ? aus.push(f) : f());
    listen("ablage-semantik", () => {
      laden();
      onGeaendert();
    })
      .then(merken)
      .catch(() => {});
    listen<{ model_id: string; percentage: number }>(
      "model-download-progress",
      (e) => {
        if (e.payload.model_id === MODELL_ID)
          setProzent(Math.round(e.payload.percentage));
      },
    )
      .then(merken)
      .catch(() => {});
    return () => {
      aktiv = false;
      aus.forEach((f) => f());
    };
  }, [laden, onGeaendert]);

  if (!s) return null;

  const schalten = async (an: boolean) => {
    setBusy(true);
    try {
      setS(await api.semantikSetzen(an));
      onGeaendert();
    } catch {
      laden();
    } finally {
      setBusy(false);
    }
  };

  const herunterladen = async () => {
    setProzent(0);
    setS({ ...s, laedt: true });
    try {
      setS(await api.semantikLaden());
      onGeaendert();
    } catch {
      laden();
    } finally {
      setProzent(null);
    }
  };

  const abbrechen = () => {
    api.semantikAbbrechen().catch(() => {});
  };

  const entfernen = async () => {
    setBusy(true);
    try {
      setS(await api.semantikLoeschen());
      onGeaendert();
    } catch {
      laden();
    } finally {
      setBusy(false);
    }
  };

  const schluessel = statusSchluessel(s);
  return (
    <div className="ablage-semantik" data-testid="ablage-semantik">
      <Sparkles size={16} aria-hidden="true" />
      <div className="ablage-semantik__text">
        <span className="ablage-semantik__titel">
          {t("ablage.semantik.titel")}
        </span>
        <span
          className="ablage-mut"
          role="status"
          data-testid="ablage-semantik-status"
        >
          {t(`ablage.semantik.status.${schluessel}`, {
            mb: s.groesse_mb,
            prozent: prozent ?? 0,
            anzahl: s.vektoren_fehlen,
          })}
          {!s.standard && s.an && ` ${t("ablage.semantik.optIn")}`}
        </span>
      </div>
      {s.an && !s.modell_bereit && !s.laedt && (
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void herunterladen()}
          data-testid="ablage-semantik-laden"
        >
          {t("ablage.semantik.laden", { mb: s.groesse_mb })}
        </Button>
      )}
      {s.laedt && (
        <Button
          variant="secondary"
          size="sm"
          onClick={abbrechen}
          data-testid="ablage-semantik-abbrechen"
        >
          {t("ablage.semantik.abbrechen")}
        </Button>
      )}
      {s.modell_bereit && !s.laedt && !s.nachziehen && (
        <Button
          variant="danger-ghost"
          size="sm"
          disabled={busy}
          onClick={() => void entfernen()}
          title={t("ablage.semantik.entfernenHinweis", { mb: s.groesse_mb })}
          data-testid="ablage-semantik-entfernen"
        >
          {t("ablage.semantik.entfernen")}
        </Button>
      )}
      <Switch
        checked={s.an}
        disabled={busy}
        onChange={(_e, d) => void schalten(d.checked)}
        aria-label={t("ablage.semantik.titel")}
        aria-description={t("ablage.semantik.hinweis")}
        title={t("ablage.semantik.hinweis")}
        data-testid="ablage-semantik-schalter"
      />
    </div>
  );
};
