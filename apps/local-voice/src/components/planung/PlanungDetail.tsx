import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ArrowLeft, Save, Send, Trash2 } from "lucide-react";
import type { Consent, Einladung, Hinweis, Plan, PlanLink } from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { api } from "./api";
import { Einwilligungen } from "./Einwilligungen";
import { EinladungsVorschau } from "./EinladungsVorschau";
import {
  beginnAusText,
  beginnTextAusMs,
  eingabeAusFormular,
  fehlerSchluessel,
  formularAusPlan,
  istAbgeschlossen,
  leeresFormular,
  uebergabeErlaubt,
  type PlanForm,
} from "./logik";
import { PlanungFormular } from "./PlanungFormular";
import { Regelhinweise } from "./Regelhinweise";
import { TerminKarte } from "./TerminKarte";

interface Props {
  /** Die Planung; `null` = neue, noch nicht gespeicherte. */
  plan: Plan | null;
  link: PlanLink | null;
  schmal: boolean;
  onZurueck: () => void;
  /** Nach Speichern, Übergabe, Bestätigung oder Löschen: Liste neu laden, ggf. `id` wählen. */
  onGeaendert: (id: string | null) => void;
}

/** Stand für „geändert?“: der Beginn gehört nicht zur Planung (er stammt aus Outlook). */
const stand = (f: PlanForm): string => JSON.stringify({ ...f, beginn: "" });

/** Wartezeit nach der letzten Eingabe, bevor Regelprüfung und Vorschau neu fragen. */
const ENTPRELL_MS = 250;

/** Das Blatt einer Planung: Formular links, Regeln, Vorschau, Termin und Einwilligungen daneben. */
export const PlanungDetail: React.FC<Props> = ({
  plan,
  link,
  schmal,
  onZurueck,
  onGeaendert,
}) => {
  const { t } = useTranslation();
  const beginnAusTermin = link?.starts_at != null;
  // Nur beim Öffnen (die Seite hängt das Blatt je Auswahl neu ein, `key`).
  const [start] = useState<PlanForm>(() =>
    plan
      ? formularAusPlan(plan, beginnTextAusMs(link?.starts_at))
      : leeresFormular(),
  );
  const [form, setForm] = useState<PlanForm>(start);
  const gespeichert = useRef(stand(start));
  const geaendert = stand(form) !== gespeichert.current;
  const [hinweise, setHinweise] = useState<Hinweis[] | null>(null);
  const [vorschau, setVorschau] = useState<Einladung | null>(null);
  const [vorschauFehler, setVorschauFehler] = useState(false);
  const [einwilligungen, setEinwilligungen] = useState<Consent[] | null>(null);
  const [beschaeftigt, setBeschaeftigt] = useState(false);
  const [loeschen, setLoeschen] = useState(false);
  const gesperrt = !!plan && istAbgeschlossen(plan.status);
  const planId = plan?.id ?? null;

  // Regelprüfung und Vorschau des Backends, kurz nach der letzten Eingabe.
  useEffect(() => {
    let veraltet = false;
    const timer = setTimeout(() => {
      const eingabe = eingabeAusFormular(form);
      Promise.all([
        api.pruefen(eingabe, beginnAusText(form.beginn)),
        api.vorschau(eingabe, planId),
      ])
        .then(([h, v]) => {
          if (veraltet) return;
          setHinweise(h);
          setVorschau(v);
          setVorschauFehler(false);
        })
        .catch(() => {
          if (!veraltet) setVorschauFehler(true);
        });
    }, ENTPRELL_MS);
    return () => {
      veraltet = true;
      clearTimeout(timer);
    };
  }, [form, planId]);

  // Stand der Einwilligungen der gespeicherten Planung.
  useEffect(() => {
    if (!planId) return;
    let veraltet = false;
    api
      .einwilligungen(planId)
      .then((c) => !veraltet && setEinwilligungen(c))
      .catch(() => !veraltet && setEinwilligungen([]));
    return () => {
      veraltet = true;
    };
  }, [planId, plan?.updated_at]);

  const erlaubt = hinweise !== null && uebergabeErlaubt(hinweise);

  const melde = (e: unknown) => {
    const roh = e instanceof Error ? e.message : String(e);
    const code = roh.split(":")[0].trim();
    toast.error(t(fehlerSchluessel(roh), { code }));
  };

  const speichern = async () => {
    setBeschaeftigt(true);
    try {
      const eingabe = eingabeAusFormular(form);
      const neu = plan
        ? await api.aendern(plan.id, eingabe)
        : await api.anlegen(eingabe);
      gespeichert.current = stand(form);
      toast.success(t("planung.aktionen.gespeichert"));
      onGeaendert(neu.id);
    } catch (e) {
      melde(e);
    } finally {
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
      onGeaendert(plan.id);
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
      onGeaendert(plan.id);
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
      onGeaendert(null);
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
            gesperrt={gesperrt}
            beginnAusTermin={beginnAusTermin}
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
          <Regelhinweise hinweise={hinweise} form={form} />
          <EinladungsVorschau einladung={vorschau} fehler={vorschauFehler} />
          <Einwilligungen
            einwilligungen={einwilligungen}
            transkriptionGeplant={form.transcription_planned}
            gespeichert={!!plan}
          />
        </aside>
      </div>
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
    </div>
  );
};
