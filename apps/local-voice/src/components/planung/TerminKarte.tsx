import React from "react";
import { useTranslation } from "react-i18next";
import type { Plan, PlanLink } from "@/bindings";
import { planungsZeitText, statusMarken } from "./logik";
import { Statusmarke } from "./Teile";

/**
 * Status der Planung und der verknüpfte Termin. Outlook führt Zeit, Ort und Teilnehmende.
 * „Verschiebung bestätigen“ steht seit lva#844 in der Meldung über der Karte (Register
 * „Termin & Zusagen“), nicht mehr in der Karte.
 */
export const TerminKarte: React.FC<{
  plan: Plan;
  link: PlanLink | null;
}> = ({ plan, link }) => {
  const { t } = useTranslation();
  const marken = statusMarken(plan, link);
  const termin =
    link?.starts_at != null
      ? `${planungsZeitText(link.starts_at)}${
          link.ends_at != null ? ` – ${planungsZeitText(link.ends_at)}` : ""
        }`
      : null;
  return (
    <section
      className="plg-karte"
      aria-labelledby="plg-termin-titel"
      data-testid="planung-termin"
    >
      <h2 id="plg-termin-titel" className="plg-karte__titel">
        {t("planung.termin.titel")}
      </h2>
      <div className="plg-marken" data-testid="planung-marken">
        {marken.map((m) => (
          <Statusmarke key={m.schluessel} marke={m} />
        ))}
      </div>
      <ul className="plg-erklaerungen">
        {marken.map((m) => (
          <li key={m.schluessel}>
            {t(`planung.statusHinweis.${m.schluessel}`)}
          </li>
        ))}
      </ul>
      {!termin && (
        <p className="plg-meldung" data-testid="planung-kein-termin">
          {t("planung.termin.keiner")}
        </p>
      )}
      {termin && link && (
        <dl className="plg-daten">
          <dt>{t("planung.termin.zeit")}</dt>
          <dd>{termin}</dd>
          {link.location && (
            <>
              <dt>{t("planung.termin.ort")}</dt>
              <dd>{link.location}</dd>
            </>
          )}
          {link.join_url && (
            <>
              <dt>{t("planung.termin.beitritt")}</dt>
              <dd className="plg-daten__lang">{link.join_url}</dd>
            </>
          )}
          {link.last_seen_at != null && (
            <>
              <dt>{t("planung.termin.zuletzt")}</dt>
              <dd>{planungsZeitText(link.last_seen_at)}</dd>
            </>
          )}
        </dl>
      )}
    </section>
  );
};
