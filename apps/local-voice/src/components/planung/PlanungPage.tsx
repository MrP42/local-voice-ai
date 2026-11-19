import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { CalendarSync, Plus } from "lucide-react";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import { api } from "./api";
import { PlanungDetail } from "./PlanungDetail";
import { PlanungListe } from "./PlanungListe";
import { usePlanung } from "./usePlanung";
import "./planung.css";

/** Auswahl „neue Planung“ (noch ohne Id). */
const NEU = "__neu__";

const SCHMAL = "(max-width: 1023px)";

function useSchmal(): boolean {
  const [schmal, setSchmal] = useState(
    () => typeof window !== "undefined" && window.matchMedia(SCHMAL).matches,
  );
  useEffect(() => {
    const q = window.matchMedia(SCHMAL);
    const f = () => setSchmal(q.matches);
    f();
    q.addEventListener("change", f);
    return () => q.removeEventListener("change", f);
  }, []);
  return schmal;
}

/**
 * Seite „Planung“ (lva#534 P2): Besprechungen vorbereiten (Schritt 1 des Ablaufs). Ziel, erwartetes
 * Ergebnis, Agenda mit Zeitboxen, Rollen, Unterlagen, Hinweise, Klassifizierung und „Transkription
 * geplant“. Outlook führt Zeit, Ort und Teilnehmende; die Seite zeigt Status und Einwilligungsstand.
 */
export const PlanungPage: React.FC = () => {
  const { t } = useTranslation();
  const daten = usePlanung();
  const schmal = useSchmal();
  const [auswahl, setAuswahl] = useState<string | null>(null);
  const [gleicht, setGleicht] = useState(false);

  const plan =
    auswahl && auswahl !== NEU
      ? (daten.plaene.find((p) => p.id === auswahl) ?? null)
      : null;
  // Gewählte Planung ist verschwunden (gelöscht): zurück zur Liste.
  useEffect(() => {
    if (auswahl && auswahl !== NEU && !daten.laden && !plan) setAuswahl(null);
  }, [auswahl, daten.laden, plan]);

  const geaendert = useCallback(
    async (id: string | null) => {
      // Erst laden, dann wählen: sonst fände die Seite die neue Planung noch nicht und gäbe die Auswahl auf.
      await daten.neuLaden();
      setAuswahl(id);
    },
    [daten.neuLaden],
  );

  const abgleichen = async () => {
    setGleicht(true);
    try {
      const b = await api.abgleichen();
      toast.success(
        t("planung.abgleich.ergebnis", {
          geprueft: b.geprueft,
          geaendert: b.geaendert,
        }),
      );
      if (b.mehrdeutig.length > 0)
        toast.warning(
          t("planung.abgleich.mehrdeutig", { count: b.mehrdeutig.length }),
        );
      await daten.neuLaden();
    } catch {
      toast.error(t("planung.abgleich.fehler"));
    } finally {
      setGleicht(false);
    }
  };

  const zeigeListe = !schmal || auswahl === null;
  const zeigeDetail = auswahl !== null && (auswahl === NEU || plan !== null);

  return (
    <PageShell
      title={t("planung.title")}
      description={t("planung.description")}
      actions={
        <>
          <Button
            type="button"
            size="sm"
            icon={<CalendarSync size={16} aria-hidden="true" />}
            disabled={gleicht}
            onClick={abgleichen}
            data-testid="planung-abgleichen"
          >
            {t("planung.abgleich.knopf")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="primary"
            icon={<Plus size={16} aria-hidden="true" />}
            onClick={() => setAuswahl(NEU)}
            data-testid="planung-neu"
          >
            {t("planung.neu")}
          </Button>
        </>
      }
    >
      <div
        className={`plg ${zeigeListe && zeigeDetail ? "plg--zwei" : ""}`}
        data-testid="planung-seite"
      >
        {zeigeListe && (
          <PlanungListe
            plaene={daten.plaene}
            links={daten.links}
            laden={daten.laden}
            fehler={daten.fehler}
            auswahl={auswahl}
            onAuswahl={setAuswahl}
          />
        )}
        {zeigeDetail && (
          <PlanungDetail
            key={auswahl ?? ""}
            plan={plan}
            link={plan ? (daten.links.get(plan.id) ?? null) : null}
            schmal={schmal}
            onZurueck={() => setAuswahl(null)}
            onGeaendert={(id) => void geaendert(id)}
          />
        )}
        {!zeigeDetail && !schmal && (
          <div className="plg-spalte plg-start" data-testid="planung-start">
            <p>{t("planung.start")}</p>
          </div>
        )}
      </div>
    </PageShell>
  );
};
