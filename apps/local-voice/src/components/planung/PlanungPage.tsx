import React, {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { OverlayDrawer } from "@fluentui/react-components";
import { CalendarPlus, CalendarSync, PanelLeft, Plus } from "lucide-react";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import { api } from "./api";
import { PlanungDetail } from "./PlanungDetail";
import { PlanungListe, type ListenFilter } from "./PlanungListe";
import { usePlanung } from "./usePlanung";
import { deepLinkTarget, type DeepLinkTarget } from "@/lib/deepLink";
import { tfApi } from "./terminfindung/api";
import { TerminfindungDialog } from "./terminfindung/TerminfindungDialog";
import "./planung.css";

/** Bereits behandelte Links: ein Wiedereinhängen der Seite öffnet denselben Link nicht erneut. */
const behandelt = new WeakSet<DeepLinkTarget>();

/** Auswahl „neue Planung“ (noch ohne Id). */
const NEU = "__neu__";

/**
 * Aufteilung nach Fenstergröße (lva#844 §1): ab 1000 px Liste und Planung nebeneinander;
 * darunter die Liste als Schublade; unter 640 px Breite oder 560 px Höhe Liste ODER Planung.
 */
const SCHMAL = "(max-width: 639px), (max-height: 559px)";
const SCHUBLADE = "(max-width: 999px)";

type Aufteilung = "breit" | "schublade" | "schmal";

function useMedien(abfrage: string): boolean {
  const [trifft, setTrifft] = useState(
    () => typeof window !== "undefined" && window.matchMedia(abfrage).matches,
  );
  useEffect(() => {
    const q = window.matchMedia(abfrage);
    const f = () => setTrifft(q.matches);
    f();
    q.addEventListener("change", f);
    return () => q.removeEventListener("change", f);
  }, [abfrage]);
  return trifft;
}

function useAufteilung(): Aufteilung {
  const schmal = useMedien(SCHMAL);
  const schublade = useMedien(SCHUBLADE);
  return schmal ? "schmal" : schublade ? "schublade" : "breit";
}

/**
 * Seite „Planung“ (lva#534 P2): Besprechungen vorbereiten (Schritt 1 des Ablaufs). Ziel, erwartetes
 * Ergebnis, Agenda mit Zeitboxen, Rollen, Unterlagen, Hinweise, Klassifizierung und „Transkription
 * geplant“. Outlook führt Zeit, Ort und Teilnehmende; die Seite zeigt Status und Einwilligungsstand.
 */
export const PlanungPage: React.FC = () => {
  const { t } = useTranslation();
  const daten = usePlanung();
  const aufteilung = useAufteilung();
  const schmal = aufteilung === "schmal";
  const [auswahl, setAuswahl] = useState<string | null>(null);
  // Filter und Suche der Liste überstehen den Wechsel zwischen Spalte und Schublade.
  const [filter, setFilter] = useState<ListenFilter>("aktiv");
  const [suche, setSuche] = useState("");
  const [schublade, setSchublade] = useState(false);
  const schubladeKnopf = useRef<HTMLButtonElement>(null);
  const sucheRef = useRef<HTMLInputElement>(null);
  const [gleicht, setGleicht] = useState(false);
  const link = useSyncExternalStore(
    deepLinkTarget.subscribe,
    deepLinkTarget.getSnapshot,
  );
  const [tfLink, setTfLink] = useState<{
    id: string;
    planId: string | null;
  } | null>(null);
  const [ohnePlanung, setOhnePlanung] = useState<string | null>(null);

  // lva#845: Link „In Local Voice planen“ – Umfrage suchen und ihre Planung öffnen.
  useEffect(() => {
    if (!link || behandelt.has(link)) return;
    behandelt.add(link);
    tfApi
      .holen(link.id)
      .then((v) => {
        if (v.poll.plan_id) {
          setTfLink({ id: v.poll.id, planId: v.poll.plan_id });
          setAuswahl(v.poll.plan_id);
        } else setOhnePlanung(v.poll.id);
      })
      .catch(() => toast.error(t("planung.terminfindung.fehler.laden")));
  }, [link, t]);

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
  // Schublade nur, solange daneben eine Planung steht; sonst ist die Liste der Inhalt.
  const alsSchublade = aufteilung === "schublade" && zeigeDetail;

  useEffect(() => {
    if (!alsSchublade) setSchublade(false);
  }, [alsSchublade]);
  // Beim Öffnen steht der Fokus in der Suche (nach dem Fokus, den Fluent beim Öffnen setzt).
  useEffect(() => {
    if (!schublade) return;
    const id = requestAnimationFrame(() => sucheRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [schublade]);

  const schubladeSchliessen = useCallback(() => {
    setSchublade(false);
    // Fokus zurück auf den Auslöser, sobald die Schublade weg ist.
    requestAnimationFrame(() => schubladeKnopf.current?.focus());
  }, []);
  const waehlen = useCallback(
    (id: string) => {
      setAuswahl(id);
      if (schublade) schubladeSchliessen();
    },
    [schublade, schubladeSchliessen],
  );

  const liste = (
    <PlanungListe
      plaene={daten.plaene}
      links={daten.links}
      laden={daten.laden}
      fehler={daten.fehler}
      auswahl={auswahl}
      onAuswahl={waehlen}
      filter={filter}
      onFilter={setFilter}
      suche={suche}
      onSuche={setSuche}
      onNeuLaden={() => void daten.neuLaden()}
      onSchliessen={alsSchublade ? schubladeSchliessen : undefined}
      sucheRef={sucheRef}
    />
  );
  const ersteAnlegen =
    !daten.laden && daten.fehler === null && daten.plaene.length === 0;

  return (
    <PageShell
      title={t("planung.title")}
      description={t("planung.description")}
      actions={
        <>
          {alsSchublade && (
            <IconButton
              ref={schubladeKnopf}
              variant="secondary"
              icon={<PanelLeft size={16} aria-hidden="true" />}
              label={t("planung.schublade.oeffnen")}
              aria-expanded={schublade}
              aria-haspopup="dialog"
              onClick={() => setSchublade(true)}
              data-testid="planung-schublade-auf"
            />
          )}
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
            // Sekundär: die eine Primäraktion je Ansicht steht in der Planung (lva#844).
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
        className={`plg ${zeigeListe && zeigeDetail && !alsSchublade ? "plg--zwei" : ""} plg--${aufteilung}`}
        data-testid="planung-seite"
      >
        {zeigeListe && !alsSchublade && liste}
        {alsSchublade && (
          <OverlayDrawer
            open={schublade}
            onOpenChange={(_e, data) => {
              if (!data.open) schubladeSchliessen();
            }}
            position="start"
            surfaceMotion={null}
            backdropMotion={null}
            aria-label={t("planung.liste.titel")}
            className="plg-schublade"
            data-testid="planung-schublade"
          >
            {liste}
          </OverlayDrawer>
        )}
        {zeigeDetail && (
          <PlanungDetail
            key={`${auswahl ?? ""}|${tfLink?.planId === auswahl ? tfLink?.id : ""}`}
            plan={plan}
            link={plan ? (daten.links.get(plan.id) ?? null) : null}
            schmal={schmal}
            onZurueck={() => setAuswahl(null)}
            onGeaendert={geaendert}
            terminfindungId={
              tfLink && tfLink.planId === auswahl ? tfLink.id : null
            }
          />
        )}
        {ohnePlanung && (
          <TerminfindungDialog
            open
            onOpenChange={(o) => !o && setOhnePlanung(null)}
            pollId={ohnePlanung}
            planId={null}
            titel=""
            dauerMin={60}
            personen={[]}
            onAngelegt={() => undefined}
            onGewaehlt={() =>
              toast.success(t("planung.terminfindung.entschieden"))
            }
          />
        )}
        {!zeigeDetail && aufteilung === "breit" && !ersteAnlegen && (
          <div className="plg-spalte plg-start" data-testid="planung-start">
            <p>{t("planung.start")}</p>
          </div>
        )}
        {!zeigeDetail && aufteilung === "breit" && ersteAnlegen && (
          <section
            className="plg-spalte plg-erste"
            data-testid="planung-erste"
            aria-labelledby="planung-erste-titel"
          >
            <h2 id="planung-erste-titel" className="plg-erste__titel">
              {t("planung.ersteAnlegen.titel")}
            </h2>
            <p className="plg-erste__text">{t("planung.ersteAnlegen.text")}</p>
            <div>
              <Button
                type="button"
                variant="primary"
                icon={<CalendarPlus size={16} aria-hidden="true" />}
                onClick={() => setAuswahl(NEU)}
                data-testid="planung-erste-neu"
              >
                {t("planung.ersteAnlegen.knopf")}
              </Button>
            </div>
          </section>
        )}
      </div>
    </PageShell>
  );
};
