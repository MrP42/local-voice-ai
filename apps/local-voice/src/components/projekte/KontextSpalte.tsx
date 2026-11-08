import React from "react";
import { useTranslation } from "react-i18next";
import type { KontextUebersicht } from "@/bindings";
import { nichtImKontext, zahl } from "@/lib/projekteKontext";
import type { Modellfenster } from "./useProjekte";

const anteil = (u: KontextUebersicht | null, f: Modellfenster): number =>
  u && f.tokens > 0
    ? Math.min(100, Math.round((u.tokens / f.tokens) * 100))
    : 0;

export const zuGross = (u: KontextUebersicht | null, f: Modellfenster) =>
  !!u && u.tokens > f.tokens;

/** Summe, Anzahl und Fenster: der obere Teil der rechten Spalte. */
export const KontextSumme: React.FC<{
  uebersicht: KontextUebersicht | null;
  fenster: Modellfenster;
}> = ({ uebersicht, fenster }) => {
  const { t } = useTranslation();
  const summe = uebersicht?.tokens ?? 0;
  const count = uebersicht?.count ?? 0;
  const gross = zuGross(uebersicht, fenster);
  return (
    <div className="pb-summe" data-testid="kontext-summe">
      <b data-testid="kontext-summe-text">
        {t("projektBereich.summe", {
          summe: zahl(summe),
          fenster: zahl(fenster.tokens),
        })}
      </b>
      <div
        className={`pb-meter${gross ? " pb-meter--zu" : ""}`}
        role="img"
        aria-label={t("projektBereich.meter", {
          prozent: anteil(uebersicht, fenster),
        })}
      >
        <i style={{ width: `${anteil(uebersicht, fenster)}%` }} />
      </div>
      <span className="pb-note" data-testid="kontext-anzahl">
        {count === 0
          ? t("projektBereich.zaehlKeine")
          : t("projektBereich.zaehl", { count })}
        {" · "}
        {fenster.modell
          ? t("projektBereich.fensterModell", { modell: fenster.modell })
          : t("projektBereich.fenster")}
      </span>
    </div>
  );
};

/** Inhalt der rechten Spalte „Kontext im Projekt“ (auch im Dialog „Details“ unter 999 px). */
export const KontextInhalt: React.FC<{
  uebersicht: KontextUebersicht | null;
  fenster: Modellfenster;
}> = ({ uebersicht, fenster }) => {
  const { t } = useTranslation();
  const fehlt = nichtImKontext(uebersicht);
  return (
    <>
      <KontextSumme uebersicht={uebersicht} fenster={fenster} />
      {fehlt.length > 0 && (
        <section className="pb-box" data-testid="nicht-im-kontext">
          <h3>{t("projektBereich.nichtImKontext")}</h3>
          <ul>
            {fehlt.map((f) => (
              <li key={f.art}>
                {t(`projektBereich.fehlt.${f.art}`, { count: f.count })}
              </li>
            ))}
          </ul>
        </section>
      )}
      <section className="pb-box">
        <h3>{t("projektBereich.geltung.titel")}</h3>
        {t("projektBereich.geltung.text")}
      </section>
      <section className="pb-box" data-testid="ki-im-projekt">
        <h3>{t("projektBereich.ki.titel")}</h3>
        <p style={{ margin: 0 }}>{t("projektBereich.ki.text")}</p>
        <p className="pb-note" style={{ margin: "6px 0 0" }}>
          {t("projektBereich.ki.gesperrt")}
        </p>
      </section>
    </>
  );
};

/** Leiste unten statt rechter Spalte (unter 999 px). */
export const Summenleiste: React.FC<{
  uebersicht: KontextUebersicht | null;
  fenster: Modellfenster;
  onDetails: () => void;
}> = ({ uebersicht, fenster, onDetails }) => {
  const { t } = useTranslation();
  const count = uebersicht?.count ?? 0;
  const gross = zuGross(uebersicht, fenster);
  return (
    <div className="pb-sumbar" data-testid="summenleiste">
      <b>
        {t("projektBereich.summeKurz", {
          summe: zahl(uebersicht?.tokens ?? 0),
        })}
      </b>
      <span
        className={`pb-meter${gross ? " pb-meter--zu" : ""}`}
        aria-hidden="true"
      >
        <i style={{ width: `${anteil(uebersicht, fenster)}%` }} />
      </span>
      <span className="pb-note">
        {t("projektBereich.eintraegeKurz", { count })}
      </span>
      <button type="button" className="pb-btn pb-btn--link" onClick={onDetails}>
        {t("projektBereich.details")}
      </button>
    </div>
  );
};
