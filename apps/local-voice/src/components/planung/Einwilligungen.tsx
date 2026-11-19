import React from "react";
import { useTranslation } from "react-i18next";
import { CircleCheck, CircleHelp, CircleX } from "lucide-react";
import type { Consent, ConsentState } from "@/bindings";
import { einwilligungenSortiert, einwilligungsStand } from "./logik";
import { zeitText } from "./Teile";

const SYMBOL = {
  zugestimmt: CircleCheck,
  offen: CircleHelp,
  abgelehnt: CircleX,
} as const satisfies Record<ConsentState, unknown>;

/**
 * Einwilligungsstand je Person: nur Anzeige. Einholen, Nachweis und Widerruf gehören zum Start
 * der Besprechung (eigenes Paket). Identität ist die E-Mail-Adresse oder die Personen-Id.
 */
export const Einwilligungen: React.FC<{
  einwilligungen: Consent[] | null;
  transkriptionGeplant: boolean;
  gespeichert: boolean;
}> = ({ einwilligungen, transkriptionGeplant, gespeichert }) => {
  const { t, i18n } = useTranslation();
  const liste = einwilligungen ? einwilligungenSortiert(einwilligungen) : [];
  const stand = einwilligungsStand(liste);
  return (
    <section
      className="plg-karte"
      aria-labelledby="plg-einw-titel"
      data-testid="planung-einwilligungen"
    >
      <h2 id="plg-einw-titel" className="plg-karte__titel">
        {t("planung.einwilligungen.titel")}
      </h2>
      <p className="plg-karte__hilfe">
        {t("planung.einwilligungen.nurAnzeige")}
      </p>
      {!transkriptionGeplant && (
        <p className="plg-meldung">
          {t("planung.einwilligungen.nichtGeplant")}
        </p>
      )}
      {transkriptionGeplant && !gespeichert && (
        <p className="plg-meldung">
          {t("planung.einwilligungen.ungespeichert")}
        </p>
      )}
      {transkriptionGeplant && gespeichert && liste.length === 0 && (
        <p className="plg-meldung" data-testid="planung-einw-leer">
          {t("planung.einwilligungen.keine")}
        </p>
      )}
      {liste.length > 0 && (
        <>
          <p className="plg-karte__zaehlung" role="status">
            {t("planung.einwilligungen.stand", stand)}
          </p>
          <ul className="plg-personen">
            {liste.map((c) => {
              const Symbol = SYMBOL[c.state];
              return (
                <li
                  key={c.person}
                  className={`plg-person plg-person--${c.state}`}
                  data-testid="planung-einw-person"
                >
                  <Symbol size={16} aria-hidden="true" />
                  <span className="plg-person__name">{c.person}</span>
                  <span className="plg-person__stand">
                    {t(`planung.einwilligungen.zustand.${c.state}`)}
                    {c.state !== "offen" && c.recorded_at != null && c.channel
                      ? t("planung.einwilligungen.nachweis", {
                          datum: zeitText(c.recorded_at, i18n.language),
                          kanal: t(`planung.einwilligungen.kanal.${c.channel}`),
                        })
                      : ""}
                  </span>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </section>
  );
};
