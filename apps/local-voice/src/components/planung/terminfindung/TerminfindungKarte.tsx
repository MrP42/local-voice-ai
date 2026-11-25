import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CalendarSearch } from "lucide-react";
import { events, type PollDefinition, type PollView } from "@/bindings";
import { Button } from "../../ui/Button";
import { tfApi } from "./api";
import { offenePersonen } from "./logik";
import { planungsZeitText } from "../logik";
import { vorschlagLang } from "./zeit";

/** Seitenkarte der Planung: Stand der Terminfindung und Einstieg in den Dialog. */
export const TerminfindungKarte: React.FC<{
  planId: string;
  /** Zuletzt angelegte oder per Link geöffnete Umfrage (Vorrang vor der Suche). */
  pollId: string | null;
  gesperrt: boolean;
  onOeffnen: (pollId: string | null) => void;
}> = ({ planId, pollId, gesperrt, onOeffnen }) => {
  const { t } = useTranslation();
  const [ansicht, setAnsicht] = useState<PollView | null>(null);

  const laden = useCallback(async () => {
    try {
      const liste: PollDefinition[] = await tfApi.liste();
      const passend =
        (pollId && liste.find((p) => p.id === pollId)) ||
        liste
          .filter((p) => p.plan_id === planId && p.status !== "abgebrochen")
          .sort((a, b) => b.frist - a.frist)[0];
      setAnsicht(passend ? await tfApi.holen(passend.id) : null);
    } catch {
      setAnsicht(null);
    }
  }, [planId, pollId]);

  useEffect(() => {
    void laden();
    const stop = events.pollUpdated.listen(() => void laden());
    return () => {
      void stop.then((f) => f());
    };
  }, [laden]);

  const poll = ansicht?.poll ?? null;
  const gefragt = poll?.teilnehmende.length ?? 0;
  const offen =
    poll && ansicht ? offenePersonen(poll, ansicht.answers).length : 0;
  const gewaehlt =
    poll?.status === "entschieden" && poll.entschieden_nr != null
      ? poll.vorschlaege.find((v) => v.nr === poll.entschieden_nr)
      : undefined;

  return (
    <section
      className="plg-karte"
      aria-labelledby="tf-karte-titel"
      data-testid="tf-karte"
    >
      <h2 id="tf-karte-titel" className="plg-karte__titel">
        {t("planung.terminfindung.karte.titel")}
      </h2>
      {poll ? (
        <>
          <p className="plg-karte__hilfe">
            <strong>
              {t(`planung.terminfindung.karte.status.${poll.status}`)}
            </strong>
            {poll.status === "offen" &&
              ` · ${t("planung.terminfindung.karte.stand", {
                da: gefragt - offen,
                von: gefragt,
                frist: planungsZeitText(poll.frist),
              })}`}
            {gewaehlt && ` · ${vorschlagLang(gewaehlt.start, gewaehlt.ende)}`}
          </p>
          <Button
            type="button"
            variant="secondary"
            icon={<CalendarSearch size={16} />}
            onClick={() => onOeffnen(poll.id)}
            data-testid="tf-karte-oeffnen"
          >
            {t("planung.terminfindung.karte.oeffnen")}
          </Button>
        </>
      ) : (
        <>
          <p className="plg-karte__hilfe">
            {t("planung.terminfindung.karte.leer")}
          </p>
          <Button
            type="button"
            variant="secondary"
            icon={<CalendarSearch size={16} />}
            disabled={gesperrt}
            onClick={() => onOeffnen(null)}
            data-testid="tf-karte-starten"
          >
            {t("planung.terminfindung.karte.starten")}
          </Button>
        </>
      )}
    </section>
  );
};
