import React from "react";
import { useTranslation } from "react-i18next";
import { CircleAlert, CircleCheck, Info, TriangleAlert } from "lucide-react";
import type { Hinweis, Stufe } from "@/bindings";
import {
  regelParameter,
  regelSchluessel,
  stufenZaehlung,
  type PlanForm,
} from "./logik";

const SYMBOL = {
  fehler: CircleAlert,
  warnung: TriangleAlert,
  info: Info,
} as const satisfies Record<Stufe, unknown>;

/** Hinweise der Regelprüfung des Backends. Nur „Fehler“ verhindert Speichern und Übergabe. */
export const Regelhinweise: React.FC<{
  hinweise: Hinweis[] | null;
  form: PlanForm;
  fehler: boolean;
}> = ({ hinweise, form, fehler }) => {
  const { t } = useTranslation();
  const z = hinweise ? stufenZaehlung(hinweise) : null;
  return (
    <section
      className="plg-karte"
      aria-labelledby="plg-regeln-titel"
      data-testid="planung-regeln"
    >
      <h2 id="plg-regeln-titel" className="plg-karte__titel">
        {t("planung.regeln.titel")}
      </h2>
      {fehler && (
        <p className="plg-meldung plg-meldung--fehler" role="alert">
          {t("planung.regeln.pruefungFehler")}
        </p>
      )}
      {!fehler && hinweise === null && (
        <p className="plg-meldung" role="status">
          {t("planung.regeln.pruefen")}
        </p>
      )}
      {hinweise !== null && hinweise.length === 0 && (
        <p className="plg-ok" data-testid="planung-regeln-ok">
          <CircleCheck size={16} aria-hidden="true" />
          {t("planung.regeln.keine")}
        </p>
      )}
      {z && hinweise && hinweise.length > 0 && (
        <p className="plg-karte__zaehlung" role="status">
          {t("planung.regeln.zaehlung", z)}
        </p>
      )}
      <ul className="plg-hinweise">
        {hinweise?.map((h) => {
          const Symbol = SYMBOL[h.stufe];
          const schluessel = regelSchluessel(h);
          return (
            <li
              key={h.code}
              className={`plg-hinweis plg-hinweis--${h.stufe}`}
              data-testid={`planung-regel-${h.code}`}
            >
              <Symbol size={16} aria-hidden="true" />
              <span>
                <span className="plg-hinweis__stufe">
                  {t(`planung.regeln.stufe.${h.stufe}`)}
                </span>
                {schluessel ? t(schluessel, regelParameter(h, form)) : h.text}
              </span>
            </li>
          );
        })}
      </ul>
    </section>
  );
};
