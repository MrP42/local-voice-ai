import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ProgressBar, Switch } from "@fluentui/react-components";
import { listen } from "@tauri-apps/api/event";
import { Sparkles } from "lucide-react";
import type { AblageSemantik } from "@/bindings";
import { Button } from "../ui/Button";
import { api } from "./api";
import { fehlerTeile } from "./logik";

/** Statuszeile der Einstellung als Text (ohne Prozent: der Live-Bereich meldet nur Zustandswechsel). */
function statusSchluessel(s: AblageSemantik): string {
  if (!s.an) return "aus";
  if (s.laedt) return "laedt";
  if (!s.modell_bereit) return "fehlt";
  if (s.ladefehler) return "fehler";
  if (s.index_fehler) return "indexfehler";
  if (s.nachziehen) return "nachziehen";
  if (s.neuaufbau) return "neuaufbau";
  if (s.vektoren_fehlen > 0) return "wartet";
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
  const [busy, setBusy] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);

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
    listen<number>("ablage-semantik-fortschritt", (e) => {
      setS((alt) => (alt ? { ...alt, laedt: true, prozent: e.payload } : alt));
    })
      .then(merken)
      .catch(() => {});
    return () => {
      aktiv = false;
      aus.forEach((f) => f());
    };
  }, [laden, onGeaendert]);

  const melden = useCallback(
    (e: unknown) => {
      const { code, text } = fehlerTeile(
        e instanceof Error ? e.message : String(e),
      );
      const suffix = (code ?? "").replace(/^semantik\./, "");
      setFehler(
        t(`ablage.semantik.fehler.${suffix || "allgemein"}`, {
          defaultValue: text || t("ablage.semantik.fehler.allgemein"),
        }),
      );
    },
    [t],
  );

  if (!s) return null;

  const schalten = async (an: boolean) => {
    setBusy(true);
    setFehler(null);
    try {
      setS(await api.semantikSetzen(an));
      onGeaendert();
    } catch (e) {
      melden(e);
      laden();
    } finally {
      setBusy(false);
    }
  };

  const herunterladen = async () => {
    setFehler(null);
    setS({ ...s, laedt: true, prozent: 0 });
    try {
      setS(await api.semantikLaden());
      onGeaendert();
    } catch (e) {
      melden(e);
      laden();
    }
  };

  const abbrechen = () => {
    api.semantikAbbrechen().catch(melden);
  };

  const entfernen = async () => {
    setBusy(true);
    setFehler(null);
    try {
      setS(await api.semantikLoeschen());
      onGeaendert();
    } catch (e) {
      melden(e);
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
            anzahl: s.vektoren_fehlen,
          })}
          {!s.standard && s.an && ` ${t("ablage.semantik.optIn")}`}
        </span>
        {s.laedt && (
          <ProgressBar
            className="ablage-semantik__fortschritt"
            value={s.prozent}
            max={100}
            thickness="medium"
            aria-label={t("ablage.semantik.fortschritt")}
            aria-valuetext={t("ablage.semantik.prozent", {
              prozent: s.prozent,
            })}
            data-testid="ablage-semantik-fortschritt"
          />
        )}
        {fehler && (
          <span
            className="ablage-semantik__fehler"
            role="alert"
            data-testid="ablage-semantik-fehler"
          >
            {fehler}
          </span>
        )}
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
      {(s.modell_bereit || s.reste) && !s.laedt && !s.nachziehen && (
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
