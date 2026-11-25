import "./styles";
import "./projektbereich.css";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Info, Plus } from "lucide-react";
import { open as dateienWaehlen } from "@tauri-apps/plugin-dialog";
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
import { meldeProjekte } from "@/lib/projekteBus";
import { kopierEnde, kopierStart, useKopiert } from "@/lib/projekteKopieren";
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
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Dialog } from "../ui/Dialog";
import { EintragZeile, type Fassung } from "./EintragZeile";
import { HinzufuegenDialog } from "./HinzufuegenDialog";
import { KontextInhalt, Summenleiste, zuGross } from "./KontextSpalte";
import { anzahlVon, useModellfenster, useProjekteStand } from "./useProjekte";
import { zahl } from "@/lib/projekteKontext";

/** Alles, was zu genau einem Projekt gehört: eine späte Antwort darf nie unter einem anderen landen. */
interface Kontext {
  suiteId: string;
  uebersicht: KontextUebersicht | null;
  gefunden: GefundeneDatei[];
  lage: JournalLage | null;
}

/**
 * Quellen eines Projekts im Projektraum des Chats (lva#365 P4b, seit lva#826 nur noch eingebettet):
 * die Einträge gruppiert nach Art mit dem Schalter „Als Kontext“, rechts „Kontext im Projekt“ (schmal
 * eine Summenleiste unten mit „Details“). Alle Zahlen eines Projekts kommen aus `projektEintraege`.
 * Die Auswahl gilt auf diesem Gerät.
 */
export const ProjektePage: React.FC<{
  /** Das Projekt des Raums. */
  raum: string;
}> = ({ raum }) => {
  const { t } = useTranslation();
  const stand = useProjekteStand();
  const fenster = useModellfenster();
  const [kontext, setKontext] = useState<Kontext | null>(null);
  const [buecher, setBuecher] = useState<LibraryBook[]>([]);
  const [neuAufbauen, setNeuAufbauen] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  const [beschaeftigt, setBeschaeftigt] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [hinzu, setHinzu] = useState(false);
  /** Öffnungszähler des Hinzufügen-Dialogs: ein später Abschluss schließt nur die Öffnung, die ihn startete. */
  const hinzuGen = useRef(0);
  const hinzuOeffnen = () => {
    hinzuGen.current += 1;
    setHinzu(true);
  };
  const hinzuSchliessen = () => {
    hinzuGen.current += 1;
    setHinzu(false);
  };
  const [details, setDetails] = useState(false);
  const [entfernen, setEntfernen] = useState<EintragSicht | null>(null);

  const projekt = stand.projekte.find((p) => p.id === raum);
  const suiteId = projekt?.id ?? null;
  const eintraege = useMemo(
    () => (suiteId ? (stand.eintraege[suiteId] ?? []) : []),
    [stand.eintraege, suiteId],
  );

  const kopiert = useKopiert(suiteId);
  const aktuell = kontext?.suiteId === suiteId ? kontext : null;
  const uebersicht = aktuell?.uebersicht ?? null;
  const gefunden = aktuell?.gefunden ?? [];
  const lage = aktuell?.lage ?? null;

  /**
   * Zähler je Projekt (Ladung, Schalt-Stand): eine Antwort zählt nur, wenn ihr Projekt noch angezeigt
   * wird und ihr Zähler passt. Ein Schalten in Projekt A verwirft so nie die Ladung von Projekt B.
   */
  const gens = useRef(new Map<string, { lade: number; schalt: number }>());
  const genVon = useCallback((s: string) => {
    let g = gens.current.get(s);
    if (!g) {
      g = { lade: 0, schalt: 0 };
      gens.current.set(s, g);
    }
    return g;
  }, []);
  /** Das gerade angezeigte Projekt; null nach dem Aushängen der Seite. */
  const aktuellRef = useRef<string | null>(null);
  const istAktuell = (s: string) => aktuellRef.current === s;
  const warteschlangen = useRef(new Map<string, Promise<void>>());
  const offen = useRef(new Map<string, number>());
  /**
   * `stand` der zuletzt übernommenen Übersicht je Projekt: Das Schalten schickt ihn als Erwartung mit
   * (`expectedStand`); hat sich die Auswahl seitdem von außen geändert, schaltet das Backend nicht.
   */
  const standVon = useRef(new Map<string, string>());

  useEffect(() => {
    aktuellRef.current = suiteId;
    setFehler(null);
    return () => {
      aktuellRef.current = null;
    };
  }, [suiteId]);

  const teilSetzen = useCallback(
    (s: string, teil: Partial<Omit<Kontext, "suiteId">>) =>
      setKontext((alt) => ({
        suiteId: s,
        uebersicht: null,
        gefunden: [],
        lage: null,
        ...(alt?.suiteId === s ? alt : {}),
        ...teil,
      })),
    [],
  );

  const laden = useCallback(
    async (s: string | null) => {
      if (!s) {
        setKontext(null);
        return;
      }
      const g = genVon(s);
      const nr = ++g.lade;
      const sg = g.schalt;
      const gueltig = () => aktuellRef.current === s && g.lade === nr;
      let meldung: string | null = null;
      const merke = (e: unknown) => {
        if (meldung === null) meldung = fehlerText(e, t);
      };
      await Promise.all([
        (async () => {
          try {
            const u = auspacken(await commands.projektKontextUebersicht(s));
            // Hat in diesem Projekt inzwischen ein Schalten begonnen oder geendet, gilt dessen Antwort.
            if (gueltig() && g.schalt === sg) {
              standVon.current.set(s, u.stand);
              teilSetzen(s, { uebersicht: u });
            }
          } catch (e) {
            merke(e);
          }
        })(),
        (async () => {
          try {
            const r = await commands.projektDateienGefunden(s);
            if (gueltig())
              teilSetzen(s, {
                gefunden:
                  r.status === "ok" && Array.isArray(r.data) ? r.data : [],
              });
          } catch {
            if (gueltig()) teilSetzen(s, { gefunden: [] });
          }
        })(),
        (async () => {
          try {
            const l = await commands.projektDateienLage(s);
            if (gueltig())
              teilSetzen(s, {
                lage: l.status === "ok" ? (l.data ?? null) : null,
              });
          } catch {
            if (gueltig()) teilSetzen(s, { lage: null });
          }
        })(),
      ]);
      if (gueltig()) setFehler(meldung);
    },
    [t, teilSetzen, genVon],
  );
  const kontextLaden = () => laden(suiteId);

  // Neu laden, sobald sich Projekt oder Eintraege aendern (Zuordnen, Entfernen, Abgleich).
  useEffect(() => {
    void laden(suiteId);
  }, [laden, suiteId, stand.eintraege]);

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

  const einreihen = useCallback((s: string, f: () => Promise<void>): void => {
    const vorher = warteschlangen.current.get(s) ?? Promise.resolve();
    warteschlangen.current.set(
      s,
      vorher.then(f).catch(() => undefined),
    );
  }, []);

  /** Je Eintrag zählt die Sperre die wartenden Änderungen; `beschaeftigt` ist die Menge der Einträge mit Zähler > 0. */
  const sperrenZaehlen = (id: string, schritt: 1 | -1) => {
    const n = (offen.current.get(id) ?? 0) + schritt;
    if (n > 0) offen.current.set(id, n);
    else offen.current.delete(id);
    setBeschaeftigt(new Set(offen.current.keys()));
  };

  /**
   * Alle Änderungen eines Projekts laufen nacheinander. Ein Form- oder Fassungswechsel liest den
   * Stand unmittelbar davor frisch vom Backend und übernimmt daraus `on`: ein ausgeschalteter
   * Eintrag bleibt aus, auch wenn die Anzeige hinterherhinkt.
   * Das Schalten schickt den `stand` der Übersicht mit, auf die es sich bezieht (P4c-1): ein
   * Schalterklick den zuletzt übernommenen, ein Form- oder Fassungswechsel den frisch gelesenen.
   * Hat sich die Auswahl dazwischen von außen geändert, schaltet das Backend nicht
   * (`projekte.stand_veraltet`); die Seite lädt dann neu und zeigt die Meldung.
   */
  const aendern = (
    e: EintragSicht,
    wunsch:
      { on: boolean } | { form: KontextForm | null; fassung: string | null },
  ) => {
    const s = suiteId;
    if (!s) return;
    // Ein Schalterklick wartet nicht auf einen laufenden Schritt desselben Eintrags; ein
    // Form- oder Fassungswechsel reiht sich ein und liest dann den Stand frisch.
    if ("on" in wunsch && offen.current.has(e.id)) return;
    sperrenZaehlen(e.id, 1);
    const g = genVon(s);
    einreihen(s, async () => {
      g.schalt += 1;
      try {
        let on: boolean;
        let form: KontextForm | null = null;
        let fassung: string | null = null;
        let erwartet: string | null = standVon.current.get(s) ?? null;
        if ("on" in wunsch) on = wunsch.on;
        else {
          const frisch = auspacken(await commands.projektKontextUebersicht(s));
          on = frisch.rows.find((z) => z.id === e.id)?.on ?? false;
          form = wunsch.form;
          fassung = wunsch.fassung;
          erwartet = frisch.stand;
        }
        const neu = auspacken(
          await commands.projektKontextSchalten(
            s,
            e.id,
            on,
            form,
            fassung,
            erwartet,
          ),
        );
        // Die Antwort ist der neue Stand des Projekts, auch wenn es nicht mehr angezeigt wird.
        standVon.current.set(s, neu.stand);
        // Wird das Projekt nicht mehr angezeigt, wird nichts übernommen (es lädt beim nächsten Öffnen).
        if (istAktuell(s)) {
          teilSetzen(s, { uebersicht: neu });
          setFehler(null);
        }
      } catch (x) {
        if (istAktuell(s)) {
          // Stand neu lesen (zeigt, was wirklich gilt), dann die Meldung: das Laden loescht alte Fehler.
          await laden(s);
          if (istAktuell(s)) setFehler(fehlerText(x, t));
        }
      } finally {
        g.schalt += 1;
        sperrenZaehlen(e.id, -1);
      }
    });
  };

  const geaendert = useCallback(() => {
    notifyMeetingsChanged();
    meldeProjekte("projekte");
  }, []);

  const bestaetigenEntfernen = async () => {
    const e = entfernen;
    const s = suiteId;
    if (!e || !s) return;
    setEntfernen(null);
    const g = gruppeVon(e);
    try {
      if (g === "dateien") {
        const sicher = auspacken(
          await commands.projektDateiEntfernen(s, dateiIdVon(e.link) ?? ""),
        );
        if (!sicher) {
          toast.warning(t("projektBereich.nichtSicherEntfernt"));
          geaendert();
          return;
        }
      } else if (g === "buecher")
        auspacken(
          await commands.projektBuchEntfernen(s, buchIdVon(e.link) ?? ""),
        );
      else if (g === "aufnahmen")
        auspacken(
          await commands.projektBesprechungEntfernen(
            s,
            besprechungIdVon(e.link) ?? "",
          ),
        );
      else if (g === "recherchen")
        auspacken(
          await commands.rechercheLoeschen(rechercheIdVon(e.link) ?? ""),
        );
      else
        auspacken(
          await commands.projektSeiteEntfernen(s, seiteIdVon(e.link) ?? ""),
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
      if (istAktuell(s)) setFehler(fehlerText(x, t));
    }
  };

  const dateienKopieren = async (): Promise<string | null> => {
    const s = suiteId;
    if (!s || !kopierStart(s)) return null;
    const og = hinzuGen.current;
    try {
      const wahl = await dateienWaehlen({ multiple: true, directory: false });
      const paths = Array.isArray(wahl) ? wahl : wahl ? [wahl] : [];
      if (paths.length === 0) return null;
      const r = auspacken(await commands.projektDateienHinzufuegen(s, paths));
      toast.success(
        t("projektBereich.dialog.kopiert", {
          count: r?.length ?? paths.length,
        }),
      );
      geaendert();
      if (istAktuell(s) && hinzuGen.current === og) setHinzu(false);
      return null;
    } catch (x) {
      return fehlerText(x, t);
    } finally {
      kopierEnde(s);
    }
  };

  const aufnehmen = async (d: GefundeneDatei) => {
    const s = suiteId;
    if (!s) return;
    try {
      auspacken(await commands.projektDateiGefundenAufnehmen(s, d.id));
      geaendert();
    } catch (x) {
      if (istAktuell(s)) setFehler(fehlerText(x, t));
    }
  };

  const bestandNeu = async () => {
    const s = suiteId;
    if (!s) return;
    setNeuAufbauen(false);
    try {
      auspacken(await commands.projektDateienNeuAufbauen(s));
      geaendert();
      await laden(s);
    } catch (x) {
      if (istAktuell(s)) setFehler(fehlerText(x, t));
    }
  };

  const zeilen = useMemo(
    () => new Map((uebersicht?.rows ?? []).map((z) => [z.id, z])),
    [uebersicht],
  );
  const gross = zuGross(uebersicht, fenster);
  const gruppen = useMemo(() => gruppiere(eintraege), [eintraege]);
  const anzahl = suiteId ? anzahlVon(stand, suiteId) : 0;
  const entfernenGruppe = entfernen ? gruppeVon(entfernen) : null;
  const entfernenName = entfernen?.title ?? t("projektBereich.ohneName");

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
            <h3 className="pb-phead__titel">{t("projektRaum.quellen")}</h3>
            <span className="pb-chip" data-testid="projekt-kopf-zahl">
              {t("projektBereich.eintraege", { count: anzahl })}
            </span>
            <div className="pb-phead__sp">
              <button
                type="button"
                className="pb-btn"
                data-testid="hinzufuegen"
                onClick={hinzuOeffnen}
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
                onClick={hinzuOeffnen}
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
                      beschaeftigt={beschaeftigt.has(e.id)}
                      onSchalten={(on) => aendern(e, { on })}
                      onForm={(form, fassung) => aendern(e, { form, fassung })}
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
      ) : null}
    </div>
  );

  return (
    <>
      <div className="pb-wrap pb-wrap--raum">
        <div className="pb pb--raum" data-testid="projekte-bereich">
          {mitte}
          {projekt && (
            <div
              className="pb-col pb-rechts"
              aria-label={t("projektBereich.rechts")}
            >
              <div className="pb-colhead">{t("projektBereich.rechts")}</div>
              <KontextInhalt
                projektId={projekt?.id}
                uebersicht={uebersicht}
                fenster={fenster}
              />
            </div>
          )}
        </div>
      </div>

      <HinzufuegenDialog
        open={hinzu}
        projekt={projekt ?? null}
        eintraege={eintraege}
        onClose={hinzuSchliessen}
        onGeaendert={geaendert}
        kopiert={kopiert}
        onDateien={dateienKopieren}
      />

      <Dialog
        contentFades={false}
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
          <KontextInhalt
            projektId={projekt?.id}
            uebersicht={uebersicht}
            fenster={fenster}
          />
        </div>
      </Dialog>

      <Dialog
        contentFades={false}
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
        contentFades={false}
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
    </>
  );
};
