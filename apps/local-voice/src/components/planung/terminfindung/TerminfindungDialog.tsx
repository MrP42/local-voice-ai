import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, RefreshCw, Send, UserPlus, X } from "lucide-react";
import {
  events,
  type PersonAvailability,
  type PollDefinition,
  type PollView,
  type Rang,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { m365Konto, tfApi, type Konto } from "./api";
import {
  adresseGueltig,
  initialen,
  istIntern,
  offenePersonen,
  stufeVon,
  vorschlagUmschalten,
  wochenbeginn,
  wochenFenster,
  zurPruefung,
  type Stufe,
} from "./logik";
import { Raster } from "./Raster";
import { AntwortMatrix, Empfehlung, ZurPruefung } from "./Antworten";
import { eingabeAusMs, msAusEingabe, uhrzeit } from "./zeit";
import "./terminfindung.css";

export interface Vorschlagsperson {
  adresse: string;
  name: string;
}

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Bestehende Umfrage; `null` = neue aus der Planung. */
  pollId: string | null;
  planId: string | null;
  titel: string;
  dauerMin: number;
  /** Teilnehmende aus dem verknüpften Termin, als Vorbelegung. */
  personen: Vorschlagsperson[];
  onAngelegt: (id: string) => void;
  /** Beginn kommt aus einem verknüpften Outlook-Termin: dann wird hier nicht entschieden. */
  outlookBestimmt?: boolean;
  /**
   * Gewählter Zeitpunkt, erst nachdem die Entscheidung dauerhaft in der Umfrage liegt
   * (`entschieden_nr`); die Planung übernimmt Beginn und Dauer in ihr Formular.
   */
  onGewaehlt: (start: number, ende: number) => void;
}

const TAG = 86_400_000;
const zeitzone = () =>
  Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";

function entwurf(p: Props, konto: Konto | null): PollDefinition {
  const domain = konto?.adresse?.split("@")[1] ?? null;
  return {
    id: "",
    plan_id: p.planId,
    titel: p.titel,
    titel_an_externe: false,
    dauer_min: p.dauerMin,
    frist: Date.now() + 2 * TAG,
    status: "entwurf",
    vorschlaege: [],
    teilnehmende: p.personen
      .filter((x) => x.adresse.toLowerCase() !== konto?.adresse?.toLowerCase())
      .map((x) => ({
        adresse: x.adresse.toLowerCase(),
        name: x.name,
        intern: istIntern(x.adresse, domain),
        pflicht: false,
      })),
  };
}

const STUFEN: Stufe[] = ["waehlen", "sammeln", "entscheiden"];

/**
 * Terminfindung (lva#845 TF5): Vorschläge mit interner Verfügbarkeit wählen, Antworten sammeln,
 * entscheiden. Versand und Postfach laufen über das Microsoft-365-Konto; jede Aktion braucht einen
 * Klick. Die App zeigt nur frei/belegt, nie Termininhalte.
 */
export const TerminfindungDialog: React.FC<Props> = (props) => {
  const {
    open,
    onOpenChange,
    pollId,
    onAngelegt,
    onGewaehlt,
    outlookBestimmt,
  } = props;
  const { t } = useTranslation();
  const [konto, setKonto] = useState<Konto | null | undefined>(undefined);
  const [ansicht, setAnsicht] = useState<PollView | null>(null);
  const [def, setDef] = useState<PollDefinition | null>(null);
  const [woche, setWoche] = useState(() => wochenbeginn(Date.now() + TAG));
  const [verfuegbar, setVerfuegbar] = useState<PersonAvailability[] | null>(
    null,
  );
  // Stand der Kalenderdaten, gebunden an Woche und Teilnehmende.
  const [vStand, setVStand] = useState<{
    schluessel: string;
    zeit: string;
  } | null>(null);
  const [vLaedt, setVLaedt] = useState(false);
  const [vFehler, setVFehler] = useState<string | null>(null);
  const [raenge, setRaenge] = useState<Rang[] | null>(null);
  const [rFehler, setRFehler] = useState(false);
  const rGeneration = useRef(0);
  const vGeneration = useRef(0);
  const [beschaeftigt, setBeschaeftigt] = useState(false);
  const [neu, setNeu] = useState({ adresse: "", name: "" });

  const id = ansicht?.poll.id ?? pollId;
  const stufe = stufeVon(ansicht?.poll ?? null);
  const poll = ansicht?.poll ?? def;
  const editierbar = stufe === "waehlen";
  const personenSchluessel = (poll?.teilnehmende ?? [])
    .map((p) => p.adresse)
    .sort()
    .join(",");
  const vSchluessel = `${woche}|${personenSchluessel}`;

  // Konto und bestehende Umfrage laden.
  useEffect(() => {
    if (!open) return;
    let weg = false;
    void (async () => {
      const k = await m365Konto().catch(() => null);
      if (weg) return;
      setKonto(k);
      if (pollId) {
        try {
          const v = await tfApi.holen(pollId);
          if (weg) return;
          setAnsicht(v);
          setDef(v.poll);
          const erster = v.poll.vorschlaege[0];
          if (erster) setWoche(wochenbeginn(erster.start));
        } catch {
          if (!weg) toast.error(t("planung.terminfindung.fehler.laden"));
        }
      } else {
        setAnsicht(null);
        setDef(entwurf(props, k));
      }
    })();
    return () => {
      weg = true;
    };
    // Vorbelegung nur beim Öffnen übernehmen (props bewusst nicht als Abhängigkeit).
  }, [open, pollId]);

  // Änderungen aus dem Postfachabruf.
  useEffect(() => {
    if (!open || !id) return;
    const stop = events.pollUpdated.listen(({ payload }) => {
      if (payload.poll_id && payload.poll_id !== id) return;
      // Alte Empfehlung sofort entwerten; entschieden wird erst wieder nach frischem Rang.
      rGeneration.current += 1;
      setRaenge(null);
      setRFehler(false);
      void tfApi
        .holen(id)
        .then(setAnsicht)
        .catch(() => setRFehler(true));
    });
    return () => {
      void stop.then((f) => f());
    };
  }, [open, id]);

  // Verfügbarkeit der Woche (Cache im Backend: 15 min).
  const ladeVerfuegbarkeit = useCallback(async () => {
    if (!id) return;
    // Nur der jüngste Abruf zählt (Wochenwechsel, geänderte Personen).
    const gen = ++vGeneration.current;
    setVLaedt(true);
    setVFehler(null);
    setVerfuegbar(null);
    setVStand(null);
    try {
      const daten = await tfApi.verfuegbarkeit(
        id,
        wochenFenster(woche, zeitzone()),
      );
      if (gen !== vGeneration.current) return;
      setVerfuegbar(daten);
      setVStand({ schluessel: vSchluessel, zeit: uhrzeit(Date.now()) });
    } catch {
      if (gen !== vGeneration.current) return;
      setVFehler("fehler");
    } finally {
      if (gen === vGeneration.current) setVLaedt(false);
    }
  }, [id, woche, vSchluessel]);
  useEffect(() => {
    if (open && id) void ladeVerfuegbarkeit();
  }, [open, id, ladeVerfuegbarkeit]);

  // Rangliste, sobald Antworten da sind. Jede neue Ansicht verwirft die alte Liste sofort; nur die
  // Antwort des jüngsten Abrufs zählt, damit nie nach einer veralteten Empfehlung entschieden wird.
  useEffect(() => {
    const gen = ++rGeneration.current;
    setRaenge(null);
    setRFehler(false);
    if (!open || !id || stufe === "waehlen") return;
    tfApi
      .rang(id)
      .then((r) => {
        if (gen === rGeneration.current) setRaenge(r);
      })
      .catch(() => {
        if (gen === rGeneration.current) setRFehler(true);
      });
  }, [open, id, stufe, ansicht]);

  const pflicht = useMemo(
    () =>
      new Set(
        (poll?.teilnehmende ?? [])
          .filter((p) => p.pflicht)
          .map((p) => p.adresse.toLowerCase()),
      ),
    [poll],
  );

  const speichern = useCallback(
    async (naechste: PollDefinition) => {
      setDef(naechste);
      if (!konto) return;
      setBeschaeftigt(true);
      try {
        const v = naechste.id
          ? await tfApi.aendern(naechste.id, naechste)
          : await tfApi.anlegen(konto.id, naechste);
        setAnsicht(v);
        setDef(v.poll);
        if (!naechste.id) onAngelegt(v.poll.id);
      } catch {
        toast.error(t("planung.terminfindung.fehler.speichern"));
      } finally {
        setBeschaeftigt(false);
      }
    },
    [konto, onAngelegt, t],
  );

  const personHinzu = () => {
    if (!poll || !adresseGueltig(neu.adresse)) return;
    const adresse = neu.adresse.trim().toLowerCase();
    if (poll.teilnehmende.some((p) => p.adresse === adresse)) return;
    const domain = konto?.adresse?.split("@")[1] ?? null;
    void speichern({
      ...poll,
      teilnehmende: [
        ...poll.teilnehmende,
        {
          adresse,
          name: neu.name.trim(),
          intern: istIntern(adresse, domain),
          pflicht: false,
        },
      ],
    });
    setNeu({ adresse: "", name: "" });
  };

  const senden = async () => {
    if (!id) return;
    setBeschaeftigt(true);
    try {
      setAnsicht(await tfApi.senden(id));
      toast.success(t("planung.terminfindung.gesendet"));
    } catch {
      toast.error(t("planung.terminfindung.fehler.senden"));
    } finally {
      setBeschaeftigt(false);
    }
  };

  const waehlen = async (nr: number) => {
    const v = poll?.vorschlaege.find((x) => x.nr === nr);
    if (!id || !v) return;
    setBeschaeftigt(true);
    try {
      if (outlookBestimmt) {
        toast.info(t("planung.terminfindung.ausOutlook"));
        return;
      }
      // Entscheidung und Nummer landen in einem Schritt dauerhaft in der Umfrage; erst danach
      // übernimmt die Planung. Scheitert das, bleibt die Umfrage offen.
      try {
        setAnsicht(await tfApi.entscheiden(id, nr));
      } catch {
        toast.error(t("planung.terminfindung.fehler.schliessen"));
        return;
      }
      onGewaehlt(v.start, v.ende);
      onOpenChange(false);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const extern = (poll?.teilnehmende ?? []).filter((p) => !p.intern).length;
  const gefragt = (poll?.teilnehmende ?? []).length;
  const offen =
    poll && ansicht ? offenePersonen(poll, ansicht.answers).length : 0;
  const pruefen = ansicht ? zurPruefung(ansicht.history) : [];
  const bester = raenge?.find((r) => !r.ausgeschlossen)?.nr ?? null;
  const zustellungen = ansicht?.deliveries ?? [];
  const ausstehend = zustellungen.filter((d) => d.state !== "sent").length;
  const weiterSendbar =
    stufe === "sammeln" &&
    zustellungen.some((d) => d.state === "pending") &&
    !beschaeftigt;
  const sendbar =
    editierbar &&
    !!id &&
    (poll?.vorschlaege.length ?? 0) >= 2 &&
    gefragt > 0 &&
    !beschaeftigt;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("planung.terminfindung.titel")}
      description={poll?.titel || undefined}
      closeLabel={t("common.close")}
      className="tf-dialog"
      contentClassName="tf-dialog__inhalt"
    >
      {konto === null && (
        <p
          className="tf-hinweis tf-hinweis--warn"
          role="status"
          data-testid="tf-ohne-konto"
        >
          {t("planung.terminfindung.ohneKonto")}
        </p>
      )}
      <ol className="tf-stufen" aria-label={t("planung.terminfindung.ablauf")}>
        {STUFEN.map((s, i) => {
          const pos = STUFEN.indexOf(stufe);
          const zustand = i < pos ? "fertig" : i === pos ? "aktiv" : "";
          return (
            <li
              key={s}
              className={`tf-stufe ${zustand && `tf-stufe--${zustand}`}`}
              aria-current={i === pos ? "step" : undefined}
            >
              <b aria-hidden>{i < pos ? <Check size={12} /> : i + 1}</b>
              {t(`planung.terminfindung.stufe.${s}`)}
            </li>
          );
        })}
        {id && <li className="tf-stufen__id">{id}</li>}
      </ol>

      {poll && stufe === "waehlen" && (
        <div className="tf-raster-layout">
          <Raster
            woche={woche}
            onWoche={setWoche}
            verfuegbar={verfuegbar}
            laedt={vLaedt}
            fehler={id ? vFehler : null}
            pflicht={pflicht}
            dauerMin={poll.dauer_min}
            vorschlaege={poll.vorschlaege}
            onZelle={
              konto && !beschaeftigt
                ? (start) =>
                    void speichern({
                      ...poll,
                      vorschlaege: vorschlagUmschalten(
                        poll.vorschlaege,
                        start,
                        poll.dauer_min,
                      ),
                    })
                : null
            }
            stand={vStand?.schluessel === vSchluessel ? vStand.zeit : null}
          />
          <section
            className="tf-karte"
            aria-labelledby="tf-personen-titel"
            data-testid="tf-personen"
          >
            <header className="tf-karte__kopf">
              <h3 id="tf-personen-titel">
                {t("planung.terminfindung.personen.titel")}
              </h3>
              <span className="tf-marke">
                {t("planung.terminfindung.personen.anzahl", { count: gefragt })}
              </span>
            </header>
            <div className="tf-karte__inhalt">
              {!id && (
                <div className="tf-hinweis">
                  <span>{t("planung.terminfindung.personen.erst")}</span>
                  <Button
                    type="button"
                    variant="primary-soft"
                    size="sm"
                    disabled={!konto || gefragt === 0 || beschaeftigt}
                    onClick={() => void speichern(poll)}
                    data-testid="tf-entwurf"
                  >
                    {t("planung.terminfindung.personen.laden")}
                  </Button>
                </div>
              )}
              <ul className="tf-personen">
                {poll.teilnehmende.map((p) => (
                  <li key={p.adresse}>
                    <span
                      className={`tf-av ${p.intern ? "" : "tf-av--extern"}`}
                      aria-hidden
                    >
                      {initialen(p.name, p.adresse)}
                    </span>
                    <span className="tf-personen__name">
                      <span>{p.name || p.adresse}</span>
                      <small>
                        {p.intern
                          ? t("planung.terminfindung.personen.intern")
                          : t("planung.terminfindung.personen.extern")}
                        {p.name ? ` · ${p.adresse}` : ""}
                      </small>
                    </span>
                    <label
                      className="tf-pflichtwahl"
                      title={t("planung.terminfindung.pflicht")}
                    >
                      <input
                        type="checkbox"
                        checked={p.pflicht}
                        disabled={beschaeftigt}
                        onChange={(e) =>
                          void speichern({
                            ...poll,
                            teilnehmende: poll.teilnehmende.map((x) =>
                              x.adresse === p.adresse
                                ? { ...x, pflicht: e.target.checked }
                                : x,
                            ),
                          })
                        }
                      />
                      {t("planung.terminfindung.personen.pflicht")}
                    </label>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      icon={<X size={14} />}
                      aria-label={t(
                        "planung.terminfindung.personen.entfernen",
                        { name: p.name || p.adresse },
                      )}
                      title={t("planung.terminfindung.personen.entfernen", {
                        name: p.name || p.adresse,
                      })}
                      disabled={beschaeftigt}
                      onClick={() =>
                        void speichern({
                          ...poll,
                          teilnehmende: poll.teilnehmende.filter(
                            (x) => x.adresse !== p.adresse,
                          ),
                        })
                      }
                    />
                  </li>
                ))}
              </ul>
              <form
                className="tf-neu"
                onSubmit={(e) => {
                  e.preventDefault();
                  personHinzu();
                }}
              >
                <input
                  type="email"
                  className="tf-eingabe"
                  placeholder={t("planung.terminfindung.personen.adresse")}
                  aria-label={t("planung.terminfindung.personen.adresse")}
                  value={neu.adresse}
                  onChange={(e) => setNeu({ ...neu, adresse: e.target.value })}
                />
                <input
                  className="tf-eingabe"
                  placeholder={t("planung.terminfindung.personen.name")}
                  aria-label={t("planung.terminfindung.personen.name")}
                  value={neu.name}
                  onChange={(e) => setNeu({ ...neu, name: e.target.value })}
                />
                <Button
                  type="submit"
                  variant="secondary"
                  icon={<UserPlus size={16} />}
                  disabled={
                    !adresseGueltig(neu.adresse) || beschaeftigt || !konto
                  }
                >
                  {t("planung.terminfindung.personen.hinzu")}
                </Button>
              </form>
              <label className="tf-feld">
                <span>{t("planung.terminfindung.frist")}</span>
                <input
                  type="datetime-local"
                  className="tf-eingabe"
                  value={eingabeAusMs(poll.frist)}
                  disabled={beschaeftigt}
                  onChange={(e) => {
                    const ms = msAusEingabe(e.target.value);
                    if (ms && ms > Date.now())
                      void speichern({ ...poll, frist: ms });
                  }}
                />
              </label>
              <label className="tf-schalter">
                <input
                  type="checkbox"
                  role="switch"
                  checked={poll.titel_an_externe}
                  disabled={beschaeftigt}
                  onChange={(e) =>
                    void speichern({
                      ...poll,
                      titel_an_externe: e.target.checked,
                    })
                  }
                />
                <span>{t("planung.terminfindung.titelExtern")}</span>
              </label>
              <p className="tf-hilfe">
                {t("planung.terminfindung.vorschlaegeHilfe", {
                  n: poll.vorschlaege.length,
                })}
              </p>
              <Button
                type="button"
                variant="primary"
                icon={<Send size={16} />}
                disabled={!sendbar}
                onClick={() => void senden()}
                data-testid="tf-senden"
              >
                {t("planung.terminfindung.senden", { count: gefragt })}
              </Button>
              {extern > 0 && (
                <p className="tf-hilfe">
                  {t("planung.terminfindung.externHinweis", { count: extern })}
                </p>
              )}
            </div>
          </section>
        </div>
      )}

      {poll && ansicht && stufe !== "waehlen" && (
        <div className="tf-raster-layout">
          <section className="tf-karte" aria-labelledby="tf-antworten-titel">
            <header className="tf-karte__kopf">
              <h3 id="tf-antworten-titel">
                {t("planung.terminfindung.matrix.titel")}
              </h3>
              <span className="tf-marke">
                {t("planung.terminfindung.matrix.stand", {
                  da: gefragt - offen,
                  von: gefragt,
                })}
              </span>
            </header>
            <div className="tf-karte__inhalt">
              <AntwortMatrix
                poll={poll}
                antworten={ansicht.answers}
                bester={bester}
              />
            </div>
          </section>
          <div className="tf-spalte">
            {ausstehend > 0 && (
              <section
                className="tf-karte"
                aria-labelledby="tf-zustellung-titel"
                data-testid="tf-zustellung"
              >
                <header className="tf-karte__kopf">
                  <h3 id="tf-zustellung-titel">
                    {t("planung.terminfindung.zustellung.titel")}
                  </h3>
                  <span className="tf-marke tf-marke--warn" role="status">
                    {t("planung.terminfindung.zustellung.teilweise", {
                      count: ausstehend,
                    })}
                  </span>
                </header>
                <div className="tf-karte__inhalt">
                  <ul className="tf-zustellung">
                    {zustellungen
                      .filter((d) => d.state !== "sent")
                      .map((d) => {
                        const p = poll.teilnehmende.find(
                          (x) => x.adresse === d.address,
                        );
                        return (
                          <li key={d.address}>
                            <span>{p?.name || d.address}</span>
                            <span
                              className={`tf-marke ${d.state === "review" ? "tf-marke--warn" : ""}`}
                            >
                              {t(
                                `planung.terminfindung.zustellung.${["pending", "sending", "review"].includes(d.state) ? d.state : "pending"}`,
                              )}
                            </span>
                          </li>
                        );
                      })}
                  </ul>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    icon={<RefreshCw size={14} />}
                    disabled={!weiterSendbar}
                    onClick={() => void senden()}
                    data-testid="tf-weiter-senden"
                  >
                    {t("planung.terminfindung.zustellung.weiter")}
                  </Button>
                </div>
              </section>
            )}
            <Empfehlung
              poll={poll}
              raenge={raenge}
              fehler={rFehler}
              offen={offen}
              beschaeftigt={
                beschaeftigt || stufe !== "sammeln" || !raenge || rFehler
              }
              onWaehlen={(nr) => void waehlen(nr)}
            />
            <ZurPruefung
              poll={poll}
              eintraege={pruefen}
              beschaeftigt={beschaeftigt}
              onUebernehmen={(h) => {
                if (!id) return;
                setBeschaeftigt(true);
                tfApi
                  .bestaetigen(id, h.id)
                  .then(setAnsicht)
                  .catch(() =>
                    toast.error(t("planung.terminfindung.fehler.speichern")),
                  )
                  .finally(() => setBeschaeftigt(false));
              }}
            />
          </div>
        </div>
      )}
    </Dialog>
  );
};
