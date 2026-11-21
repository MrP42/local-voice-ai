import React from "react";
import { useTranslation } from "react-i18next";
import { CircleCheck, CircleHelp, CircleX } from "lucide-react";
import type { Consent, ConsentState } from "@/bindings";
import { Button } from "../ui/Button";
import {
  einwilligungenSortiert,
  einwilligungsStand,
  planungsZeitText,
} from "./logik";

const SYMBOL = {
  zugestimmt: CircleCheck,
  offen: CircleHelp,
  abgelehnt: CircleX,
} as const satisfies Record<ConsentState, unknown>;

/**
 * Einwilligungsstand je Person mit optionalen Aktionen. Identität ist die E-Mail-Adresse oder
 * die Personen-Id; Widerruf setzt den Stand auf offen und erhält den Nachweis im Verlauf.
 */
export const Einwilligungen: React.FC<{
  einwilligungen: Consent[] | null;
  transkriptionGeplant: boolean;
  gespeichert: boolean;
  fehler: boolean;
  onErneutVersuchen: () => void;
  onSetzen?: (person: string, state: ConsentState) => Promise<void>;
  beschaeftigt?: boolean;
}> = ({
  einwilligungen,
  transkriptionGeplant,
  gespeichert,
  fehler,
  onErneutVersuchen,
  onSetzen,
  beschaeftigt = false,
}) => {
  const { t } = useTranslation();
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
        {t(
          onSetzen
            ? "planung.einwilligungen.verwalten"
            : "planung.einwilligungen.nurAnzeige",
        )}
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
      {gespeichert && fehler && (
        <div className="plg-meldung plg-meldung--fehler" role="alert">
          <p>{t("planung.einwilligungen.ladeFehler")}</p>
          <Button type="button" onClick={onErneutVersuchen}>
            {t("planung.einwilligungen.erneutVersuchen")}
          </Button>
        </div>
      )}
      {gespeichert && !fehler && einwilligungen === null && (
        <p className="plg-meldung" role="status">
          {t("planung.einwilligungen.laden")}
        </p>
      )}
      {transkriptionGeplant &&
        gespeichert &&
        !fehler &&
        einwilligungen !== null &&
        liste.length === 0 && (
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
                          datum: planungsZeitText(c.recorded_at),
                          kanal: t(`planung.einwilligungen.kanal.${c.channel}`),
                        })
                      : ""}
                  </span>
                  {onSetzen && (
                    <div className="flex flex-wrap gap-2">
                      {(["zugestimmt", "abgelehnt", "offen"] as const).map(
                        (state) => (
                          <Button
                            key={state}
                            type="button"
                            size="sm"
                            disabled={beschaeftigt || c.state === state}
                            onClick={() => {
                              // Die aufrufende Seite meldet Fehler ohne Inhaltsdaten.
                              void onSetzen(c.person, state).catch(() => {});
                            }}
                            data-testid={`planung-einw-${state}`}
                          >
                            {t(`planung.einwilligungen.aktion.${state}`)}
                          </Button>
                        ),
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      )}
    </section>
  );
};
