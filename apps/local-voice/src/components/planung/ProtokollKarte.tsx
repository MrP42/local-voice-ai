import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { Erreicht, Plan, ProtokollStand } from "@/bindings";
import { minutesErrorCode, minutesErrorDetail } from "@/lib/meetingMinutes";
import { Button } from "../ui/Button";
import { defaultDocBasis } from "../settings/meetings/language/DocBasisFields";
import { api } from "./api";
import { ProtokollEntwurf } from "./ProtokollEntwurf";
import {
  ERREICHT_WERTE,
  angebot,
  fehlerMeldung,
  protokollFehlerSchluessel,
} from "./protokoll";

interface Props {
  plan: Plan;
  /** Nach einer Änderung, die den Status der Planung berührt (Freigabe): Liste neu laden. */
  onGeaendert: () => void;
}

/**
 * Protokoll der Planung (lva#736, P5): Protokoll aus der Agenda erzeugen (vorhandener Weg der
 * KI-Notizen mit der Vorlage aus der Agenda), „Ziel erreicht“ von Hand setzen, das Protokoll
 * freigeben und einen Entwurf an die Teilnehmenden erzeugen. Entscheidungen trifft immer ein Mensch;
 * versendet wird nichts automatisch.
 */
export const ProtokollKarte: React.FC<Props> = ({ plan, onGeaendert }) => {
  const { t, i18n } = useTranslation();
  const [stand, setStand] = useState<ProtokollStand | null>(null);
  const [ladefehler, setLadefehler] = useState(false);
  const [laeuft, setLaeuft] = useState<
    null | "erzeugen" | "erreicht" | "freigeben"
  >(null);
  const [fehler, setFehler] = useState<string | null>(null);
  const [entwurfOffen, setEntwurfOffen] = useState(false);
  const sperre = useRef(false);
  const abruf = useRef(0);

  const laden = useCallback(async () => {
    const mein = ++abruf.current;
    try {
      const s = await api.protokollStand(plan.id);
      if (mein === abruf.current) {
        setStand(s);
        setLadefehler(false);
      }
    } catch {
      if (mein === abruf.current) setLadefehler(true);
    }
  }, [plan.id]);

  useEffect(() => {
    void laden();
  }, [laden, plan.status]);

  /** Genau ein Vorgang zugleich; Fehler werden als Text gezeigt, nie mit Inhalten. */
  const vorgang = async (
    art: "erzeugen" | "erreicht" | "freigeben",
    arbeit: () => Promise<void>,
    fehlerText: (roh: string) => string,
  ) => {
    if (sperre.current) return;
    sperre.current = true;
    setLaeuft(art);
    setFehler(null);
    try {
      await arbeit();
    } catch (e) {
      setFehler(fehlerText(fehlerMeldung(e)));
    } finally {
      sperre.current = false;
      setLaeuft(null);
      await laden();
    }
  };

  const erzeugen = () =>
    vorgang(
      "erzeugen",
      async () => {
        const v = await api.protokollVorlage(plan.id);
        await api.protokollErzeugen(
          v.meeting_id,
          v.template_id,
          defaultDocBasis(i18n.language),
        );
        toast.success(t("planung.protokoll.erzeugt"));
      },
      (roh) => {
        const planFehler = protokollFehlerSchluessel(roh);
        if (planFehler !== "planung.protokoll.fehler.allgemein") {
          return t(planFehler);
        }
        const code = minutesErrorCode(roh);
        return t(`meetings.minutes.errors.${code}`, {
          error: minutesErrorDetail(roh),
          defaultValue: t("planung.protokoll.fehler.allgemein"),
        });
      },
    );

  const erreichtSetzen = (wert: Erreicht | null) =>
    vorgang(
      "erreicht",
      async () => {
        setStand(await api.erreichtSetzen(plan.id, wert));
      },
      (roh) => t(protokollFehlerSchluessel(roh)),
    );

  const freigeben = () =>
    vorgang(
      "freigeben",
      async () => {
        setStand(await api.protokollFreigeben(plan.id));
        onGeaendert();
        toast.success(t("planung.protokoll.freigegebenToast"));
      },
      (roh) => t(protokollFehlerSchluessel(roh)),
    );

  const a = angebot(stand);
  const beschaeftigt = laeuft !== null;
  const erreicht = stand?.erreicht ?? null;
  return (
    <section
      className="plg-karte"
      aria-labelledby="plg-protokoll-titel"
      data-testid="planung-protokoll"
    >
      <h2 id="plg-protokoll-titel" className="plg-karte__titel">
        {t("planung.protokoll.titel")}
      </h2>
      <p className="plg-karte__hilfe">{t("planung.protokoll.hinweis")}</p>
      {ladefehler && (
        <p className="plg-meldung plg-meldung--fehler" role="alert">
          {t("planung.protokoll.ladefehler")}
        </p>
      )}
      {stand && !stand.meeting_id && (
        <p
          className="plg-meldung"
          role="status"
          data-testid="protokoll-ohne-besprechung"
        >
          {t("planung.protokoll.ohneBesprechung")}
        </p>
      )}
      <Button
        type="button"
        variant="primary"
        disabled={!a.erzeugen || beschaeftigt}
        onClick={() => void erzeugen()}
        data-testid="protokoll-erzeugen"
      >
        {laeuft === "erzeugen"
          ? t("planung.protokoll.erzeugtLaeuft")
          : t("planung.protokoll.aus")}
      </Button>
      {stand?.dokument_id && (
        <p className="plg-karte__hilfe" data-testid="protokoll-version">
          {t("planung.protokoll.version", { version: stand.dokument_version })}
        </p>
      )}

      <fieldset
        className="plg-feld"
        disabled={beschaeftigt}
        data-testid="protokoll-erreicht"
      >
        <legend className="plg-feld__label">
          {t("planung.protokoll.erreicht.titel")}
        </legend>
        <div
          role="radiogroup"
          aria-label={t("planung.protokoll.erreicht.titel")}
        >
          {ERREICHT_WERTE.map((wert) => (
            <label key={wert} className="plg-erreicht__wahl">
              <input
                type="radio"
                name={`erreicht-${plan.id}`}
                checked={erreicht === wert}
                onChange={() => void erreichtSetzen(wert)}
                data-testid={`protokoll-erreicht-${wert}`}
              />
              {t(`planung.protokoll.erreicht.${wert}`)}
            </label>
          ))}
        </div>
        {erreicht === null ? (
          <p className="plg-feld__hilfe" data-testid="protokoll-erreicht-offen">
            {t("planung.protokoll.erreicht.offen")}
          </p>
        ) : (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void erreichtSetzen(null)}
            data-testid="protokoll-erreicht-zurueck"
          >
            {t("planung.protokoll.erreicht.zurueck")}
          </Button>
        )}
        {stand?.kopf_fehlt && (
          <p className="plg-feld__hilfe" data-testid="protokoll-kopf-fehlt">
            {t("planung.protokoll.erreicht.kopfFehlt")}
          </p>
        )}
      </fieldset>

      <div className="plg-karte__aktionen">
        <Button
          type="button"
          disabled={!a.freigeben || beschaeftigt}
          onClick={() => void freigeben()}
          data-testid="protokoll-freigeben"
        >
          {t("planung.protokoll.freigeben")}
        </Button>
        <Button
          type="button"
          disabled={!a.entwurf || beschaeftigt}
          title={a.entwurf ? undefined : t("planung.protokoll.entwurfGesperrt")}
          onClick={() => setEntwurfOffen(true)}
          data-testid="protokoll-entwurf-oeffnen"
        >
          {t("planung.protokoll.entwurfKnopf")}
        </Button>
      </div>
      {stand?.freigegeben && (
        <p className="plg-ok" role="status" data-testid="protokoll-freigegeben">
          {t("planung.protokoll.istFreigegeben")}
        </p>
      )}
      {stand?.geaendert_seit_freigabe && (
        <p
          className="plg-meldung"
          role="status"
          data-testid="protokoll-geaendert"
        >
          {t("planung.protokoll.geaendertSeitFreigabe")}
        </p>
      )}
      {fehler && (
        <p
          className="plg-meldung plg-meldung--fehler"
          role="alert"
          data-testid="protokoll-fehler"
        >
          {fehler}
        </p>
      )}
      <ProtokollEntwurf
        open={entwurfOffen}
        onOpenChange={setEntwurfOffen}
        planId={plan.id}
      />
    </section>
  );
};
