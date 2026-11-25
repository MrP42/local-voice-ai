import React, { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, CircleCheck, CircleHelp, CircleX } from "lucide-react";
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
 *
 * Aufklappkarte (lva#844 R2): Kopf mit Zählung und Meldungen bleibt sichtbar; Personen und der
 * Verlauf (`children`, eigene Karte direkt nach dieser) bleiben zugeklappt im DOM und werden nur
 * mit `hidden` ausgeblendet. Kein Fluent-`AccordionPanel`, das zugeklappt nichts rendert.
 */
export const Einwilligungen: React.FC<{
  einwilligungen: Consent[] | null;
  transkriptionGeplant: boolean;
  gespeichert: boolean;
  fehler: boolean;
  onErneutVersuchen: () => void;
  onSetzen?: (person: string, state: ConsentState) => Promise<void>;
  beschaeftigt?: boolean;
  /** Gesetzt: die Karte klappt auf (z. B. wenn der Verlauf einen Fehler meldet). */
  aufklappen?: boolean;
  /** Zugeklappte Karte nach dieser, z. B. der Verlauf; der Aufklappknopf steuert beide. */
  children?: React.ReactNode;
}> = ({
  einwilligungen,
  transkriptionGeplant,
  gespeichert,
  fehler,
  onErneutVersuchen,
  onSetzen,
  beschaeftigt = false,
  aufklappen = false,
  children,
}) => {
  const { t } = useTranslation();
  const [offen, setOffen] = useState(false);
  const inhaltId = useId();
  const nachherId = useId();
  useEffect(() => {
    if (aufklappen) setOffen(true);
  }, [aufklappen]);
  const liste = einwilligungen ? einwilligungenSortiert(einwilligungen) : [];
  const stand = einwilligungsStand(liste);
  return (
    <>
      <section
        className="plg-karte"
        aria-labelledby="plg-einw-titel"
        data-testid="planung-einwilligungen"
      >
        <h2 className="plg-karte__titel plg-aufklapp__titel">
          <button
            type="button"
            id="plg-einw-titel"
            className="plg-aufklapp__knopf"
            aria-expanded={offen}
            aria-controls={children ? `${inhaltId} ${nachherId}` : inhaltId}
            onClick={() => setOffen((o) => !o)}
          >
            <ChevronRight
              size={16}
              aria-hidden="true"
              className="plg-aufklapp__pfeil"
            />
            {t("planung.einwilligungen.titel")}
          </button>
        </h2>
        {liste.length > 0 && (
          <p className="plg-karte__zaehlung" role="status">
            {t("planung.einwilligungen.stand", stand)}
          </p>
        )}
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
        <div id={inhaltId} className="plg-aufklapp__inhalt" hidden={!offen}>
          <p className="plg-karte__hilfe">
            {t(
              onSetzen
                ? "planung.einwilligungen.verwalten"
                : "planung.einwilligungen.nurAnzeige",
            )}
          </p>
          {liste.length > 0 && (
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
                            kanal: t(
                              `planung.einwilligungen.kanal.${c.channel}`,
                            ),
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
          )}
        </div>
      </section>
      {children && (
        <div id={nachherId} className="plg-aufklapp__nachher" hidden={!offen}>
          {children}
        </div>
      )}
    </>
  );
};
