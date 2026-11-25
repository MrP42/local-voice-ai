import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ArrowLeft, Save, Send, Trash2 } from "lucide-react";
import type {
  CalEvent,
  Consent,
  Einladung,
  EinwilligungsEintrag,
  Hinweis,
  Plan,
  PlanLink,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { api } from "./api";
import { Einwilligungen } from "./Einwilligungen";
import { EinwilligungsVerlauf } from "./EinwilligungsVerlauf";
import { AgendaZeitboxen } from "./AgendaZeitboxen";
import { EinladungsVorschau } from "./EinladungsVorschau";
import {
  beginnAusText,
  beginnTextAusMs,
  eingabeAusFormular,
  fehlerSchluessel,
  formularAusPlan,
  formularGueltig,
  istAbgeschlossen,
  leeresFormular,
  uebergabeErlaubt,
  type PlanForm,
} from "./logik";
import { PlanungFormular } from "./PlanungFormular";
import { ProtokollKarte } from "./ProtokollKarte";
import { Regelhinweise } from "./Regelhinweise";
import { LaufPanel } from "./LaufPanel";
import { StartPanel, type StartErgebnis } from "./StartPanel";
import { TerminKarte } from "./TerminKarte";
import { TerminfindungKarte } from "./terminfindung/TerminfindungKarte";
import { TerminfindungDialog } from "./terminfindung/TerminfindungDialog";

interface Props {
  /** Die Planung; `null` = neue, noch nicht gespeicherte. */
  plan: Plan | null;
  link: PlanLink | null;
  schmal: boolean;
  onZurueck: () => void;
  /** Nach Speichern, Übergabe, Bestätigung oder Löschen: Liste neu laden, ggf. `id` wählen. */
  onGeaendert: (id: string | null) => Promise<void>;
  /** Per Link geöffnete Terminfindung (lva#845); öffnet den Dialog einmal. */
  terminfindungId?: string | null;
}

/** Stand für „geändert?“: der Beginn gehört nicht zur Planung (er stammt aus Outlook). */
const stand = (f: PlanForm): string => JSON.stringify({ ...f, beginn: "" });

/** Wartezeit nach der letzten Eingabe, bevor Regelprüfung und Vorschau neu fragen. */
const ENTPRELL_MS = 250;

/** Das Blatt einer Planung: Formular links, Regeln, Vorschau, Termin und Einwilligungen daneben. */
export const PlanungDetail: React.FC<Props> = ({
  plan: geladenerPlan,
  link,
  schmal,
  onZurueck,
  onGeaendert,
  terminfindungId = null,
}) => {
  const { t } = useTranslation();
  const [tfOffen, setTfOffen] = useState(terminfindungId !== null);
  const [tfId, setTfId] = useState<string | null>(terminfindungId);
  // Die zurückgegebene Planung gilt sofort, auch solange die Liste noch lädt oder fehlschlägt.
  const [angelegterPlan, setAngelegterPlan] = useState<Plan | null>(null);
  const plan = geladenerPlan ?? angelegterPlan;
  const beginnAusTermin = link?.starts_at != null;
  // Nur beim Öffnen (die Seite hängt das Blatt je Auswahl neu ein, `key`).
  const [start] = useState<PlanForm>(() =>
    plan
      ? formularAusPlan(plan, beginnTextAusMs(link?.starts_at))
      : leeresFormular(),
  );
  const [lokalesFormular, setForm] = useState<PlanForm>(start);
  const kalenderBeginn = beginnAusTermin ? beginnTextAusMs(link?.starts_at) : null;
  const form = useMemo(
    () =>
      kalenderBeginn === null
        ? lokalesFormular
        : { ...lokalesFormular, beginn: kalenderBeginn },
    [lokalesFormular, kalenderBeginn],
  );
  const gespeichert = useRef(stand(start));
  const geaendert = stand(form) !== gespeichert.current;
  const [pruefung, setPruefung] = useState<{
    stand: string;
    hinweise: Hinweis[] | null;
    fehler: boolean;
  } | null>(null);
  const [vorschau, setVorschau] = useState<Einladung | null>(null);
  const [vorschauFehler, setVorschauFehler] = useState(false);
  const [einwilligungen, setEinwilligungen] = useState<Consent[] | null>(null);
  const [einwilligungenFehler, setEinwilligungenFehler] = useState(false);
  const [einwilligungenVersuch, setEinwilligungenVersuch] = useState(0);
  const [verlauf, setVerlauf] = useState<EinwilligungsEintrag[] | null>(null);
  const [verlaufFehler, setVerlaufFehler] = useState(false);
  const [verlaufVersuch, setVerlaufVersuch] = useState(0);
  const verlaufAbruf = useRef(0);
  const [termin, setTermin] = useState<CalEvent | null>(null);
  const [startOffen, setStartOffen] = useState(false);
  const [lauf, setLauf] = useState<StartErgebnis | null>(null);
  const [einwilligungSchreibt, setEinwilligungSchreibt] = useState(false);
  const [einwilligungFehler, setEinwilligungFehler] = useState(false);
  const einwilligungSperre = useRef(false);
  const einwilligungenAbruf = useRef(0);
  const [beschaeftigt, setBeschaeftigt] = useState(false);
  const speichert = useRef(false);
  const [loeschen, setLoeschen] = useState(false);
  const gesperrt = !!plan && istAbgeschlossen(plan.status);
  const planId = plan?.id ?? null;
  const pruefStand = JSON.stringify({ form, planId });
  // Schon im Rendern sperren: ein Effect allein käme nach der geänderten Eingabe zu spät.
  const hinweise = pruefung?.stand === pruefStand ? pruefung.hinweise : null;
  const pruefFehler = pruefung?.stand === pruefStand && pruefung.fehler;

  // Regelprüfung und Vorschau des Backends, kurz nach der letzten Eingabe.
  useEffect(() => {
    let veraltet = false;
    setPruefung(null);
    setVorschau(null);
    setVorschauFehler(false);
    const timer = setTimeout(() => {
      const eingabe = eingabeAusFormular(form);
      Promise.all([
        api.pruefen(eingabe, beginnAusText(form.beginn)),
        api.vorschau(eingabe, planId),
      ])
        .then(([h, v]) => {
          if (veraltet) return;
          setPruefung({ stand: pruefStand, hinweise: h, fehler: false });
          setVorschau(v);
          setVorschauFehler(false);
        })
        .catch(() => {
          if (veraltet) return;
          setPruefung({ stand: pruefStand, hinweise: null, fehler: true });
          setVorschau(null);
          setVorschauFehler(true);
        });
    }, ENTPRELL_MS);
    return () => {
      veraltet = true;
      clearTimeout(timer);
    };
  }, [form, planId, pruefStand]);

  // Stand der Einwilligungen der gespeicherten Planung.
  useEffect(() => {
    const abruf = ++einwilligungenAbruf.current;
    setEinwilligungen(null);
    setEinwilligungenFehler(false);
    if (!planId) return;
    let veraltet = false;
    api
      .einwilligungen(planId)
      .then((c) => {
        if (!veraltet && abruf === einwilligungenAbruf.current)
          setEinwilligungen(c);
      })
      .catch(() => {
        if (!veraltet && abruf === einwilligungenAbruf.current)
          setEinwilligungenFehler(true);
      });
    return () => {
      veraltet = true;
    };
  }, [planId, plan?.updated_at, einwilligungenVersuch]);

  // Der verknüpfte Termin liefert die Teilnehmenden und den Schlüssel für den Start.
  useEffect(() => {
    let veraltet = false;
    setTermin(null);
    if (!planId || !link) return;
    api
      .termine(24 * 60)
      .then((termine) => {
        if (veraltet) return;
        const gefunden = Array.isArray(termine)
          ? termine.find(
              (ev) =>
                !ev.cancelled &&
                (ev.uid === link.ical_uid ||
                  (link.remote_id != null && ev.uid === link.remote_id)) &&
                (link.starts_at == null || ev.starts_at === link.starts_at),
            )
          : null;
        setTermin(gefunden ?? null);
      })
      .catch(() => {
        if (!veraltet) setTermin(null);
      });
    return () => {
      veraltet = true;
    };
  }, [planId, link]);

  // Verlauf der Einwilligungen; späte Antworten dürfen den neuen Stand nicht ersetzen.
  useEffect(() => {
    const abruf = ++verlaufAbruf.current;
    setVerlauf(null);
    setVerlaufFehler(false);
    if (!planId) return;
    let veraltet = false;
    api
      .einwilligungsVerlauf(planId)
      .then((eintraege) => {
        if (!veraltet && abruf === verlaufAbruf.current) setVerlauf(eintraege);
      })
      .catch(() => {
        if (!veraltet && abruf === verlaufAbruf.current) setVerlaufFehler(true);
      });
    return () => {
      veraltet = true;
    };
  }, [planId, verlaufVersuch]);

  const neuLaden = () => {
    setEinwilligungenVersuch((v) => v + 1);
    setVerlaufVersuch((v) => v + 1);
  };

  const erlaubt =
    formularGueltig(form) && hinweise !== null && uebergabeErlaubt(hinweise);

  const einwilligungSetzen = async (
    person: string,
    state: Consent["state"],
  ) => {
    if (!planId || einwilligungSperre.current) return;
    einwilligungSperre.current = true;
    ++einwilligungenAbruf.current;
    setEinwilligungSchreibt(true);
    setEinwilligungFehler(false);
    try {
      await api.einwilligungSetzen({
        plan_id: planId,
        person,
        state,
        channel: "muendlich_bei_start",
        recorded_at: Date.now(),
      });
      setVerlaufVersuch((v) => v + 1);
      const abruf = ++einwilligungenAbruf.current;
      // Den nachgewiesenen Stand neu laden; keine optimistische Freigabe im Client.
      setEinwilligungen(null);
      setEinwilligungenFehler(false);
      try {
        const aktuell = await api.einwilligungen(planId);
        if (abruf === einwilligungenAbruf.current) setEinwilligungen(aktuell);
      } catch {
        if (abruf === einwilligungenAbruf.current) setEinwilligungenFehler(true);
      }
    } catch {
      setEinwilligungFehler(true);
    } finally {
      einwilligungSperre.current = false;
      setEinwilligungSchreibt(false);
    }
  };

  const melde = (e: unknown) => {
    const roh = e instanceof Error ? e.message : String(e);
    const code = roh.split(":")[0].trim();
    toast.error(t(fehlerSchluessel(roh), { code }));
  };

  const speichern = async () => {
    if (speichert.current || !erlaubt || gesperrt) return;
    speichert.current = true;
    setBeschaeftigt(true);
    try {
      const eingabe = eingabeAusFormular(form);
      const neu = plan
        ? await api.aendern(plan.id, eingabe)
        : await api.anlegen(eingabe);
      gespeichert.current = stand(form);
      if (!plan) setAngelegterPlan(neu);
      toast.success(t("planung.aktionen.gespeichert"));
      await onGeaendert(neu.id);
    } catch (e) {
      melde(e);
    } finally {
      speichert.current = false;
      setBeschaeftigt(false);
    }
  };

  const uebergeben = async () => {
    if (!plan) return;
    setBeschaeftigt(true);
    try {
      const e = await api.einladung(plan.id, null);
      try {
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/html": new Blob([e.html], { type: "text/html" }),
            "text/plain": new Blob([e.text], { type: "text/plain" }),
          }),
        ]);
        toast.success(t("planung.aktionen.uebergeben"));
      } catch {
        toast.warning(t("planung.aktionen.kopierenFehler"));
      }
      await onGeaendert(plan.id);
    } catch (e) {
      melde(e);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const bestaetigen = async () => {
    if (!plan) return;
    setBeschaeftigt(true);
    try {
      await api.verschiebungBestaetigen(plan.id);
      toast.success(t("planung.termin.bestaetigt"));
      await onGeaendert(plan.id);
    } catch (e) {
      melde(e);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const endgueltigLoeschen = async () => {
    if (!plan) return;
    setBeschaeftigt(true);
    try {
      await api.loeschen(plan.id);
      setLoeschen(false);
      toast.success(t("planung.aktionen.geloescht"));
      await onGeaendert(null);
    } catch (e) {
      melde(e);
    } finally {
      setBeschaeftigt(false);
    }
  };

  const uebergabeGesperrt =
    !plan || geaendert || !erlaubt || gesperrt || plan.status !== "entwurf";

  return (
    <div className="plg-spalte plg-detail" data-testid="planung-detail">
      <div className="plg-kopf plg-kopf--detail">
        {schmal && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            icon={<ArrowLeft size={16} aria-hidden="true" />}
            onClick={onZurueck}
            data-testid="planung-zurueck"
          >
            {t("planung.aktionen.zurueck")}
          </Button>
        )}
        <h2>
          {form.title.trim() ||
            (plan ? t("planung.liste.ohneTitel") : t("planung.neuTitel"))}
        </h2>
        <div className="plg-kopf__aktionen">
          {plan && (
            <Button
              type="button"
              variant="danger-ghost"
              size="sm"
              icon={<Trash2 size={16} aria-hidden="true" />}
              disabled={beschaeftigt}
              onClick={() => setLoeschen(true)}
              data-testid="planung-loeschen"
            >
              {t("planung.aktionen.loeschen")}
            </Button>
          )}
          <Button
            type="button"
            size="sm"
            icon={<Send size={16} aria-hidden="true" />}
            disabled={uebergabeGesperrt || beschaeftigt}
            title={t("planung.aktionen.uebergebenHilfe")}
            onClick={uebergeben}
            data-testid="planung-uebergeben"
          >
            {t("planung.aktionen.uebergebenKnopf")}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            icon={<Save size={16} aria-hidden="true" />}
            disabled={
              gesperrt || beschaeftigt || !erlaubt || (!!plan && !geaendert)
            }
            onClick={speichern}
            data-testid="planung-speichern"
          >
            {t("planung.aktionen.speichern")}
          </Button>
        </div>
      </div>
      {gesperrt && (
        <p className="plg-meldung" role="status">
          {t("planung.aktionen.gesperrt")}
        </p>
      )}
      <div className="plg-detail__inhalt">
        <div className="plg-detail__form">
          <PlanungFormular
            form={form}
            onChange={setForm}
            gesperrt={gesperrt || beschaeftigt}
            beginnAusTermin={beginnAusTermin}
            teilnehmende={termin?.attendees}
          />
        </div>
        <aside
          className="plg-detail__seite"
          aria-label={t("planung.seitenleiste")}
        >
          {plan && (
            <TerminKarte
              plan={plan}
              link={link}
              beschaeftigt={beschaeftigt}
              onVerschiebungBestaetigen={bestaetigen}
            />
          )}
          {plan && (
            <TerminfindungKarte
              planId={plan.id}
              pollId={tfId}
              gesperrt={gesperrt}
              onOeffnen={(id) => {
                setTfId(id);
                setTfOffen(true);
              }}
            />
          )}
          <Regelhinweise hinweise={hinweise} form={form} fehler={pruefFehler} />
          <EinladungsVorschau einladung={vorschau} fehler={vorschauFehler} />
          <Einwilligungen
            einwilligungen={einwilligungen}
            transkriptionGeplant={form.transcription_planned}
            gespeichert={!!plan}
            fehler={einwilligungenFehler}
            onErneutVersuchen={() => setEinwilligungenVersuch((v) => v + 1)}
            onSetzen={plan ? einwilligungSetzen : undefined}
            beschaeftigt={einwilligungSchreibt}
          />
          {einwilligungFehler && (
            <p className="plg-meldung plg-meldung--fehler" role="alert">
              {t("planung.einwilligungen.schreibFehler")}
            </p>
          )}
          {plan && (
            <EinwilligungsVerlauf
              eintraege={verlauf}
              fehler={verlaufFehler}
              onErneutVersuchen={() => setVerlaufVersuch((v) => v + 1)}
            />
          )}
          {plan && (
            <ProtokollKarte
              key={plan.id}
              plan={plan}
              onGeaendert={() => void onGeaendert(plan.id)}
            />
          )}
          {plan && (
            <section className="plg-karte" data-testid="planung-durchfuehren">
              <h2 className="plg-karte__titel">
                {t("planung.durchfuehren.titel")}
              </h2>
              {lauf === null ? (
                <>
                  <p className="plg-karte__hilfe">{t("planung.durchfuehren.hinweis")}</p>
                  <Button
                    type="button"
                    variant="primary"
                    data-testid="planung-durchfuehren-start"
                    disabled={gesperrt}
                    onClick={() => setStartOffen(true)}
                  >
                    {t("planung.durchfuehren.starten")}
                  </Button>
                  <AgendaZeitboxen key={plan.id} agenda={plan.agenda} />
                </>
              ) : (
                <>
                  <LaufPanel
                    key={lauf.meetingId}
                    plan={plan}
                    lauf={lauf}
                    einwilligungen={einwilligungen}
                    onEinwilligungGeaendert={neuLaden}
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    data-testid="planung-durchfuehren-schliessen"
                    onClick={() => {
                      setLauf(null);
                      neuLaden();
                    }}
                  >
                    {t("planung.durchfuehren.anzeigeSchliessen")}
                  </Button>
                </>
              )}
            </section>
          )}
        </aside>
      </div>
      <Dialog
        open={startOffen && !!plan}
        onOpenChange={(offen) => {
          setStartOffen(offen);
          if (!offen) neuLaden();
        }}
        title={t("planung.beginnen.titel")}
        closeLabel={t("planung.beginnen.abbrechen")}
        className="max-w-2xl"
      >
        {plan && (
          <StartPanel
            key={`${plan.id}:${termin?.key ?? ""}`}
            plan={plan}
            eventKey={termin?.key ?? null}
            teilnehmende={termin?.attendees ?? []}
            onGestartet={(e) => {
              setLauf(e);
              setStartOffen(false);
              neuLaden();
              toast.success(
                t(
                  e.art === "transkription"
                    ? "planung.durchfuehren.gestartetMit"
                    : "planung.durchfuehren.gestartetNotizen",
                ),
              );
            }}
            onAbbrechen={() => {
              setStartOffen(false);
              neuLaden();
            }}
          />
        )}
      </Dialog>
      <Dialog
        open={loeschen}
        onOpenChange={setLoeschen}
        title={t("planung.loeschenDialog.frage", {
          titel: plan?.title.trim() || t("planung.liste.ohneTitel"),
        })}
        closeLabel={t("planung.loeschenDialog.abbrechen")}
        footer={
          <>
            <Button
              type="button"
              variant="danger"
              disabled={beschaeftigt}
              onClick={endgueltigLoeschen}
              data-testid="planung-loeschen-ja"
            >
              {t("planung.loeschenDialog.ja")}
            </Button>
            <Button type="button" onClick={() => setLoeschen(false)}>
              {t("planung.loeschenDialog.abbrechen")}
            </Button>
          </>
        }
      >
        <p>{t("planung.loeschenDialog.text")}</p>
      </Dialog>
      {plan && tfOffen && (
        <TerminfindungDialog
          open={tfOffen}
          onOpenChange={setTfOffen}
          pollId={tfId}
          planId={plan.id}
          titel={form.title}
          dauerMin={Number(form.duration_min) || plan.duration_min}
          personen={(termin?.attendees ?? [])
            .filter((a) => a.email && !a.is_self)
            .map((a) => ({ adresse: a.email ?? "", name: a.name ?? "" }))}
          onAngelegt={setTfId}
          outlookBestimmt={beginnAusTermin}
          onGewaehlt={(beginn, ende) => {
            setForm((f) => ({
              ...f,
              beginn: beginnTextAusMs(beginn),
              duration_min: String(Math.round((ende - beginn) / 60_000)),
            }));
            toast.success(t("planung.terminfindung.uebernommen"));
          }}
        />
      )}
    </div>
  );
};
