import React from "react";
import { useTranslation } from "react-i18next";
import type { Plan, PlanLink } from "@/bindings";
import { Button } from "../ui/Button";
import { statusMarken, verschiebungOffen } from "./logik";
import { Statusmarke, terminText, zeitText } from "./Teile";

/** Status der Planung und der verknüpfte Termin. Outlook führt Zeit, Ort und Teilnehmende. */
export const TerminKarte: React.FC<{
  plan: Plan;
  link: PlanLink | null;
  beschaeftigt: boolean;
  onVerschiebungBestaetigen: () => void;
}> = ({ plan, link, beschaeftigt, onVerschiebungBestaetigen }) => {
  const { t, i18n } = useTranslation();
  const marken = statusMarken(plan, link);
  const termin = terminText(link, i18n.language);
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
              <dd>{zeitText(link.last_seen_at, i18n.language)}</dd>
            </>
          )}
        </dl>
      )}
      {verschiebungOffen(link) && (
        <Button
          type="button"
          size="sm"
          variant="primary"
          disabled={beschaeftigt}
          onClick={onVerschiebungBestaetigen}
          data-testid="planung-verschiebung-bestaetigen"
        >
          {t("planung.termin.bestaetigen")}
        </Button>
      )}
    </section>
  );
};
