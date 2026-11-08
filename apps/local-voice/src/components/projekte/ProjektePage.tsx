import "./styles";
import "./projektbereich.css";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ArrowLeft, FolderOpen, Info, Plus } from "lucide-react";
import { toast } from "sonner";
import {
  commands,
  type EintragSicht,
  type KontextForm,
  type KontextUebersicht,
  type LibraryBook,
  type GefundeneDatei,
  type JournalLage,
} from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { meldeProjekte, projektAnfrageFrisch } from "@/lib/projekteBus";
import {
  besprechungIdVon,
  buchIdVon,
  dateiIdVon,
  gruppeVon,
  gruppiere,
  rechercheIdVon,
  seiteIdVon,
} from "@/lib/projekteKontext";
import { editionsOf, langName } from "@/lib/library/editions";
import { usePersistentState } from "@/hooks/usePersistentState";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { PageShell } from "../ui/PageShell";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import { EintragZeile, type Fassung } from "./EintragZeile";
import { HinzufuegenDialog } from "./HinzufuegenDialog";
import { KontextInhalt, Summenleiste, zuGross } from "./KontextSpalte";
import { anzahlVon, useModellfenster, useProjekteStand } from "./useProjekte";
import { zahl } from "@/lib/projekteKontext";

const KLASSEN = ["PUBLIC", "INTERNAL", "PERSONAL", "SPECIAL", "RESTRICTED"];

const schmal = () =>
  typeof window !== "undefined" &&
  window.matchMedia("(max-width: 620px)").matches;

/**
 * Bereich „Projekte“ (lva#365 P4b): links die Projekte mit Zahl, in der Mitte die Projektseite
 * gruppiert nach Art mit dem Schalter „Als Kontext“ je Eintrag, rechts „Kontext im Projekt“ (unter
 * 999 px eine Summenleiste unten). Alle Zahlen eines Projekts kommen aus `projektEintraege`.
 * Die Auswahl gilt auf diesem Gerät; Chat und Sprachgespräch im Projekt bleiben bis P4c gesperrt.
 */
export const ProjektePage: React.FC = () => {
  const { t } = useTranslation();
  const stand = useProjekteStand();
  const fenster = useModellfenster();
  const [gemerkt, setGemerkt] = usePersistentState<string>(
    "projekte-auswahl",
    "",
  );
  const [mobilListe, setMobilListe] = useState<boolean>(
    () => !projektAnfrageFrisch() && schmal(),
  );
  const [uebersicht, setUebersicht] = useState<KontextUebersicht | null>(null);
  const [gefunden, setGefunden] = useState<GefundeneDatei[]>([]);
  const [buecher, setBuecher] = useState<LibraryBook[]>([]);
  const [lage, setLage] = useState<JournalLage | null>(null);
  const [neuAufbauen, setNeuAufbauen] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  const [schaltetId, setSchaltetId] = useState<string | null>(null);
  const [hinzu, setHinzu] = useState(false);
  const [details, setDetails] = useState(false);
  const [entfernen, setEntfernen] = useState<EintragSicht | null>(null);
  const [neu, setNeu] = useState(false);
  const [neuerTitel, setNeuerTitel] = useState("");
  const [neueKlasse, setNeueKlasse] = useState("PERSONAL");

  const projekt =
    stand.projekte.find((p) => p.id === gemerkt) ??
    (schmal() ? undefined : stand.projekte[0]);
  const suiteId = projekt?.id ?? null;
  const eintraege = useMemo(
    () => (suiteId ? (stand.eintraege[suiteId] ?? []) : []),
    [stand.eintraege, suiteId],
  );

  const kontextLaden = useCallback(async () => {
    if (!suiteId) {
      setUebersicht(null);
      setGefunden([]);
      setLage(null);
      return;
    }
    try {
      setUebersicht(
        auspacken(await commands.projektKontextUebersicht(suiteId)),
      );
      const g = await commands.projektDateienGefunden(suiteId);
      setGefunden(g.status === "ok" && Array.isArray(g.data) ? g.data : []);
      const l = await commands.projektDateienLage(suiteId);
      setLage(l.status === "ok" ? (l.data ?? null) : null);
      setFehler(null);
    } catch (e) {
      setFehler(fehlerText(e, t));
    }
  }, [suiteId, t]);

  // Neu laden, sobald sich Projekt oder Eintraege aendern (Zuordnen, Entfernen, Abgleich).
  useEffect(() => {
    void kontextLaden();
  }, [kontextLaden, stand.eintraege]);

  useEffect(() => {
    commands
      .libraryList()
      .then((r) =>
        setBuecher(r.status === "ok" && Array.isArray(r.data) ? r.data : []),
      )
      .catch(() => setBuecher([]));
  }, [stand.eintraege]);

  const fassungenVon = useCallback(
    (e: EintragSicht): Fassung[] => {
      const id = buchIdVon(e.link);
      const buch = id ? buecher.find((b) => b.id === id) : undefined;
      return buch
        ? editionsOf(buch).map((ed) => ({
            id: ed.id,
            label: langName(ed.lang) || ed.title,
          }))
        : [];
    },
    [buecher],
  );

  const auswaehlen = useCallback(
    (id: string) => {
      setGemerkt(id);
      setMobilListe(false);
    },
    [setGemerkt],
  );

  // Ein Klick in der Leiste von Vorlesen oder Sprachgespraech, waehrend diese Seite offen ist.
  useEffect(() => {
    const h = (e: Event) => {
      const d = (e as CustomEvent<{ section?: string; projektId?: string }>)
        .detail;
      if (d?.section === "projekte" && d.projektId) auswaehlen(d.projektId);
    };
    window.addEventListener("lv-navigate", h);
    return () => window.removeEventListener("lv-navigate", h);
  }, [auswaehlen]);

  const schalten = async (
    e: EintragSicht,
    on: boolean,
    form: KontextForm | null,
    fassung: string | null,
  ) => {
    if (!suiteId) return;
    setSchaltetId(e.id);
    try {
      setUebersicht(
        auspacken(
          await commands.projektKontextSchalten(
            suiteId,
            e.id,
            on,
            form,
            fassung,
          ),
        ),
      );
      setFehler(null);
    } catch (x) {
      // Stand neu lesen (zeigt, was wirklich gilt), dann die Meldung: das Laden loescht alte Fehler.
      await kontextLaden();
      setFehler(fehlerText(x, t));
    } finally {
      setSchaltetId(null);
    }
  };

  const geaendert = useCallback(() => {
    notifyMeetingsChanged();
    meldeProjekte("projekte");
  }, []);

  const bestaetigenEntfernen = async () => {
    const e = entfernen;
    if (!e || !suiteId) return;
    setEntfernen(null);
    const g = gruppeVon(e);
    try {
      if (g === "dateien") {
        const sicher = auspacken(
          await commands.projektDateiEntfernen(
            suiteId,
            dateiIdVon(e.link) ?? "",
          ),
        );
        if (!sicher) toast.warning(t("projektBereich.nichtSicherEntfernt"));
      } else if (g === "buecher")
        auspacken(
          await commands.projektBuchEntfernen(suiteId, buchIdVon(e.link) ?? ""),
        );
      else if (g === "aufnahmen")
        auspacken(
          await commands.projektBesprechungEntfernen(
            suiteId,
            besprechungIdVon(e.link) ?? "",
          ),
        );
      else if (g === "recherchen")
        auspacken(
          await commands.rechercheLoeschen(rechercheIdVon(e.link) ?? ""),
        );
      else
        auspacken(
          await commands.projektSeiteEntfernen(
            suiteId,
            seiteIdVon(e.link) ?? "",
          ),
        );
      toast.success(
        t(
          g === "recherchen"
            ? "projektBereich.geloescht"
            : "projektBereich.entfernt",
        ),
      );
      geaendert();
    } catch (x) {
      setFehler(fehlerText(x, t));
    }
  };

  const aufnehmen = async (d: GefundeneDatei) => {
    if (!suiteId) return;
    try {
      auspacken(await commands.projektDateiGefundenAufnehmen(suiteId, d.id));
      geaendert();
    } catch (x) {
      setFehler(fehlerText(x, t));
    }
  };

  const bestandNeu = async () => {
    if (!suiteId) return;
    setNeuAufbauen(false);
    try {
      auspacken(await commands.projektDateienNeuAufbauen(suiteId));
      geaendert();
      await kontextLaden();
    } catch (x) {
      setFehler(fehlerText(x, t));
    }
  };

  const anlegen = async () => {
    try {
      const p = auspacken(
        await commands.projektAnlegen(neuerTitel.trim(), neueKlasse, false),
      );
      setNeu(false);
      setNeuerTitel("");
      auswaehlen(p.id);
      geaendert();
    } catch (x) {
      setFehler(fehlerText(x, t));
    }
  };

  const zeilen = useMemo(
    () => new Map((uebersicht?.rows ?? []).map((z) => [z.id, z])),
    [uebersicht],
  );
  const gross = zuGross(uebersicht, fenster);
  const gruppen = useMemo(() => gruppiere(eintraege), [eintraege]);
  const anzahl = suiteId ? anzahlVon(stand, suiteId) : 0;
  const klassenOptionen = KLASSEN.map((k) => ({
    value: k,
    label: t(`projekte.klassen.${k}`),
  }));
  const entfernenGruppe = entfernen ? gruppeVon(entfernen) : null;
  const entfernenName = entfernen?.title ?? t("projektBereich.ohneName");

  const liste = (
    <div className="pb-col pb-links" aria-label={t("projektBereich.liste")}>
      <div className="pb-colhead">
        <span className="pb-lbl">{t("projektBereich.liste")}</span>
        <span className="pb-colhead__sp">
          <button
            type="button"
            className="pb-ib"
            aria-label={t("projektBereich.neuesProjekt")}
            title={t("projektBereich.neuesProjekt")}
            data-testid="projekt-neu"
            onClick={() => setNeu(true)}
          >
            <Plus width={18} height={18} aria-hidden="true" />
          </button>
        </span>
      </div>
      <nav aria-label={t("projektBereich.liste")}>
        {stand.projekte.map((p) => (
          <button
            key={p.id}
            type="button"
            className="pb-row"
            data-testid="projekt-zeile"
            data-projekt={p.id}
            aria-current={p.id === suiteId ? "true" : undefined}
            aria-label={p.title}
            title={p.title}
            onClick={() => auswaehlen(p.id)}
          >
            <FolderOpen width={18} height={18} aria-hidden="true" />
            <span className="pb-row__nm pb-lbl">{p.title}</span>
            <span className="pb-row__ct pb-lbl" data-testid="projekt-zahl">
              {anzahlVon(stand, p.id)}
            </span>
          </button>
        ))}
      </nav>
    </div>
  );

  const banner = (
    <>
      {fehler && (
        <div className="pb-banner pb-banner--warn" role="alert">
          <AlertTriangle width={18} height={18} aria-hidden="true" />
          <span className="pb-banner__tx">{fehler}</span>
        </div>
      )}
      {lage && (
        <div
          className="pb-banner pb-banner--warn"
          role="status"
          data-testid="banner-lage"
          data-lage={lage}
        >
          <AlertTriangle width={18} height={18} aria-hidden="true" />
          <span className="pb-banner__tx">
            {t(`projektBereich.lage.${lage}`)}
          </span>
          {lage !== "neuer" && (
            <span className="pb-banner__acts">
              <button
                type="button"
                className="pb-btn"
                data-testid="lage-knopf"
                onClick={() =>
                  lage === "nicht_lesbar"
                    ? void kontextLaden()
                    : setNeuAufbauen(true)
                }
              >
                {t(
                  lage === "nicht_lesbar"
                    ? "projektBereich.lage.erneut"
                    : "projektBereich.lage.neuAufbauen",
                )}
              </button>
            </span>
          )}
        </div>
      )}
      {uebersicht?.reset && (
        <div
          className="pb-banner pb-banner--warn"
          role="status"
          data-testid="banner-reset"
        >
          <AlertTriangle width={18} height={18} aria-hidden="true" />
          <span className="pb-banner__tx">
            {t("projektBereich.banner.reset")}
          </span>
        </div>
      )}
      {gross && uebersicht && (
        <div
          className="pb-banner pb-banner--warn"
          role="status"
          data-testid="banner-zugross"
        >
          <AlertTriangle width={18} height={18} aria-hidden="true" />
          <span className="pb-banner__tx">
            {t("projektBereich.banner.zuGross", {
              summe: zahl(uebersicht.tokens),
              fenster: zahl(fenster.tokens),
            })}
          </span>
        </div>
      )}
      {gefunden.map((d) => (
        <div
          key={d.id}
          className="pb-banner"
          role="status"
          data-testid="banner-gefunden"
        >
          <Info width={18} height={18} aria-hidden="true" />
          <span className="pb-banner__tx">
            {t("projektBereich.banner.gefunden")} <b>{d.name}</b>
            {t("projektBereich.banner.nichtAufgenommen")}
          </span>
          <span className="pb-banner__acts">
            <button
              type="button"
              className="pb-btn"
              onClick={() => void aufnehmen(d)}
            >
              {t("projektBereich.banner.aufnehmen")}
            </button>
          </span>
        </div>
      ))}
    </>
  );

  const mitte = (
    <div
      className="pb-col pb-mitte"
      aria-label={projekt?.title ?? t("projektBereich.titel")}
    >
      {projekt ? (
        <>
          <div className="pb-phead">
            <button
              type="button"
              className="pb-ib pb-zurueck"
              aria-label={t("projektBereich.zurueck")}
              onClick={() => setMobilListe(true)}
            >
              <ArrowLeft width={18} height={18} aria-hidden="true" />
            </button>
            <h2 className="pb-phead__titel">{projekt.title}</h2>
            <span className="pb-chip" data-testid="projekt-kopf-zahl">
              {t("projektBereich.eintraege", { count: anzahl })}
            </span>
            <div className="pb-phead__sp">
              <button
                type="button"
                className="pb-btn"
                data-testid="hinzufuegen"
                onClick={() => setHinzu(true)}
              >
                <Plus width={16} height={16} aria-hidden="true" />
                {t("projektBereich.hinzufuegen")}
              </button>
            </div>
          </div>
          {banner}
          {anzahl === 0 ? (
            <div className="pb-leer" data-testid="projekt-leer">
              <h2>{t("projektBereich.leer.titel", { name: projekt.title })}</h2>
              <p>{t("projektBereich.leer.text")}</p>
              <button
                type="button"
                className="pb-btn pb-btn--pri"
                onClick={() => setHinzu(true)}
              >
                {t("projektBereich.hinzufuegen")}
              </button>
            </div>
          ) : (
            <div className="pb-liste" data-testid="projekt-liste">
              <div className="pb-lkopf" aria-hidden="true">
                <span>{t("projektBereich.kopf.eintrag")}</span>
                <span className="pb-ent__art">
                  {t("projektBereich.kopf.art")}
                </span>
                <span>{t("projektBereich.kopf.kontext")}</span>
                <span />
              </div>
              {gruppen.map((g) => (
                <section key={g.gruppe} aria-labelledby={`pb-g-${g.gruppe}`}>
                  <h3
                    className="pb-gh"
                    id={`pb-g-${g.gruppe}`}
                    data-testid="gruppe"
                  >
                    {t(`projektBereich.gruppen.${g.gruppe}`)}
                    <span>
                      {g.gruppe === "dateien"
                        ? t("projektBereich.kopien", {
                            count: g.eintraege.length,
                          })
                        : g.eintraege.length}
                    </span>
                  </h3>
                  {g.eintraege.map((e) => (
                    <EintragZeile
                      key={e.id}
                      eintrag={e}
                      zeile={zeilen.get(e.id)}
                      fassungen={fassungenVon(e)}
                      beschaeftigt={schaltetId === e.id}
                      onSchalten={(on, form, fassung) =>
                        void schalten(e, on, form, fassung)
                      }
                      onEntfernen={() => setEntfernen(e)}
                    />
                  ))}
                </section>
              ))}
            </div>
          )}
          <Summenleiste
            uebersicht={uebersicht}
            fenster={fenster}
            onDetails={() => setDetails(true)}
          />
        </>
      ) : stand.geladen ? (
        <div className="pb-leer" data-testid="projekte-keins">
          <h2>{t("projektBereich.keins.titel")}</h2>
          <p>{t("projektBereich.keins.text")}</p>
          <button
            type="button"
            className="pb-btn pb-btn--pri"
            onClick={() => setNeu(true)}
          >
            {t("projektBereich.neuesProjekt")}
          </button>
        </div>
      ) : null}
    </div>
  );

  return (
    <PageShell
      title={t("projektBereich.titel")}
      description={t("projektBereich.beschreibung")}
      fill
    >
      <div className="pb-wrap">
        <div
          className={`pb${mobilListe ? " pb--liste" : ""}${projekt ? "" : " pb--zwei"}`}
          data-testid="projekte-bereich"
        >
          {liste}
          {mitte}
          {projekt && (
            <div
              className="pb-col pb-rechts"
              aria-label={t("projektBereich.rechts")}
            >
              <div className="pb-colhead">{t("projektBereich.rechts")}</div>
              <KontextInhalt uebersicht={uebersicht} fenster={fenster} />
            </div>
          )}
        </div>
      </div>

      <HinzufuegenDialog
        open={hinzu}
        projekt={projekt ?? null}
        eintraege={eintraege}
        onClose={() => setHinzu(false)}
        onGeaendert={geaendert}
      />

      <Dialog
        open={details}
        onOpenChange={setDetails}
        title={t("projektBereich.rechts")}
        closeLabel={t("projektBereich.dialog.schliessen")}
        footer={
          <button
            type="button"
            className="pb-btn"
            onClick={() => setDetails(false)}
          >
            {t("projektBereich.dialog.schliessen")}
          </button>
        }
      >
        <div className="pb-dlg" data-testid="kontext-details">
          <KontextInhalt uebersicht={uebersicht} fenster={fenster} />
        </div>
      </Dialog>

      <Dialog
        open={!!entfernen}
        onOpenChange={(o) => !o && setEntfernen(null)}
        title={t(
          entfernenGruppe === "recherchen"
            ? "projektBereich.entfernen.titelRecherche"
            : "projektBereich.entfernen.titel",
          { name: entfernenName },
        )}
        closeLabel={t("projektBereich.abbrechen")}
        footer={
          <>
            <button
              type="button"
              className="pb-btn pb-btn--pri"
              data-testid="entfernen-bestaetigen"
              onClick={() => void bestaetigenEntfernen()}
            >
              {t(
                entfernenGruppe === "recherchen"
                  ? "projektBereich.entfernen.loeschen"
                  : "projektBereich.entfernen.nehmen",
              )}
            </button>
            <button
              type="button"
              className="pb-btn"
              onClick={() => setEntfernen(null)}
            >
              {t("projektBereich.abbrechen")}
            </button>
          </>
        }
      >
        <div className="pb-dlg" data-testid="entfernen-text">
          {entfernenGruppe &&
            t(`projektBereich.entfernen.text.${entfernenGruppe}`, {
              projekt: projekt?.title ?? "",
            })}
        </div>
      </Dialog>

      <Dialog
        open={neuAufbauen}
        onOpenChange={setNeuAufbauen}
        title={t("projektBereich.lage.dialogTitel")}
        closeLabel={t("projektBereich.abbrechen")}
        footer={
          <>
            <button
              type="button"
              className="pb-btn pb-btn--pri"
              data-testid="neu-aufbauen-bestaetigen"
              onClick={() => void bestandNeu()}
            >
              {t("projektBereich.lage.neuAufbauen")}
            </button>
            <button
              type="button"
              className="pb-btn"
              onClick={() => setNeuAufbauen(false)}
            >
              {t("projektBereich.abbrechen")}
            </button>
          </>
        }
      >
        <div className="pb-dlg">{t("projektBereich.lage.dialogText")}</div>
      </Dialog>

      <Dialog
        open={neu}
        onOpenChange={setNeu}
        title={t("projektBereich.neuesProjekt")}
        closeLabel={t("projektBereich.abbrechen")}
        footer={
          <>
            <button
              type="button"
              className="pb-btn pb-btn--pri"
              disabled={neuerTitel.trim() === ""}
              onClick={() => void anlegen()}
            >
              {t("projektBereich.anlegen")}
            </button>
            <button
              type="button"
              className="pb-btn"
              onClick={() => setNeu(false)}
            >
              {t("projektBereich.abbrechen")}
            </button>
          </>
        }
      >
        <div className="pb-dlg" style={{ display: "grid", gap: 12 }}>
          <label>
            <span className="pb-note">{t("projektBereich.projektName")}</span>
            <Input
              value={neuerTitel}
              maxLength={200}
              aria-label={t("projektBereich.projektName")}
              onChange={(e) => setNeuerTitel(e.target.value)}
            />
          </label>
          <div>
            <span className="pb-note">{t("projektBereich.klasse")}</span>
            <Select
              value={neueKlasse}
              options={klassenOptionen}
              ariaLabel={t("projektBereich.klasse")}
              onChange={(v) => v && setNeueKlasse(v)}
              menuPortal
              isClearable={false}
            />
          </div>
        </div>
      </Dialog>
    </PageShell>
  );
};
