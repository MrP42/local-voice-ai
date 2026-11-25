import React from "react";
import { useTranslation } from "react-i18next";
import type {
  Antwort,
  PollDefinition,
  PollHistory,
  Rang,
  Wert,
} from "@/bindings";
import { Button } from "../../ui/Button";
import { aktuelleAntworten, summe, zelle } from "./logik";
import { uhrzeit, tagMonat, wochentag, vorschlagKurz } from "./zeit";

const STERN = "★";

const ZEICHEN: Record<Wert, string> = {
  passt: "✓",
  notfalls: "~",
  passt_nicht: "✕",
  offen: "?",
};

/** Matrix Person × Vorschlag mit Quelle jeder Stimme; Kalender-Werte interner Personen eingeschlossen. */
export const AntwortMatrix: React.FC<{
  poll: PollDefinition;
  antworten: Antwort[];
  bester: number | null;
}> = ({ poll, antworten, bester }) => {
  const { t } = useTranslation();
  const aktuell = aktuelleAntworten(antworten);
  return (
    <div className="tf-matrix-rahmen">
      <table className="tf-matrix" data-testid="tf-matrix">
        <caption className="tf-sr">
          {t("planung.terminfindung.matrix.caption")}
        </caption>
        <thead>
          <tr>
            <th scope="col">{t("planung.terminfindung.matrix.person")}</th>
            {poll.vorschlaege.map((v) => (
              <th
                scope="col"
                key={v.nr}
                className={v.nr === bester ? "tf-matrix--best" : undefined}
              >
                {t("planung.terminfindung.matrix.vorschlag", { nr: v.nr })}
                <b>
                  {wochentag(v.start)} {tagMonat(v.start)}
                </b>
                {uhrzeit(v.start)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {poll.teilnehmende.map((p) => (
            <tr key={p.adresse}>
              <th scope="row">
                <span className="tf-matrix__name">
                  {p.name || p.adresse}
                  {p.pflicht && (
                    <abbr
                      className="tf-pflicht"
                      title={t("planung.terminfindung.pflicht")}
                    >
                      {STERN}
                    </abbr>
                  )}
                </span>
              </th>
              {poll.vorschlaege.map((v) => {
                const z = zelle(aktuell, p.adresse, v.nr);
                const wert = z.sicher ? z.wert : "offen";
                const quelle = z.quelle
                  ? t(`planung.terminfindung.quelle.${z.quelle}`)
                  : null;
                return (
                  <td
                    key={v.nr}
                    className={v.nr === bester ? "tf-matrix--best" : undefined}
                  >
                    <span
                      className={`tf-wert tf-wert--${wert}${z.quelle === "kalender" ? " tf-wert--kalender" : ""}`}
                      aria-label={`${t(`planung.terminfindung.wert.${wert}`)}${quelle ? `, ${quelle}` : ""}`}
                      title={`${t(`planung.terminfindung.wert.${wert}`)}${quelle ? ` · ${quelle}` : ""}`}
                    >
                      {ZEICHEN[wert]}
                    </span>
                    {quelle && (
                      <span className="tf-quelle" aria-hidden>
                        {quelle}
                      </span>
                    )}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <th scope="row">{t("planung.terminfindung.matrix.passt")}</th>
            {poll.vorschlaege.map((v) => {
              const s = summe(aktuell, poll.teilnehmende, v.nr);
              return (
                <td
                  key={v.nr}
                  className={v.nr === bester ? "tf-matrix--best" : undefined}
                >
                  {s.passt} / {s.geantwortet}
                  {s.notfalls > 0 ? ` + ${s.notfalls} ~` : ""}
                </td>
              );
            })}
          </tr>
        </tfoot>
      </table>
    </div>
  );
};

/** Erkannte, aber unsichere Antworten: nichts zählt ungeprüft. */
export const ZurPruefung: React.FC<{
  poll: PollDefinition;
  eintraege: PollHistory[];
  beschaeftigt: boolean;
  onUebernehmen: (h: PollHistory) => void;
}> = ({ poll, eintraege, beschaeftigt, onUebernehmen }) => {
  const { t } = useTranslation();
  const name = (adresse: string | null) =>
    poll.teilnehmende.find(
      (p) => p.adresse.toLowerCase() === adresse?.toLowerCase(),
    )?.name ||
    adresse ||
    t("planung.terminfindung.pruefung.unbekannt");
  return (
    <section
      className="tf-karte"
      aria-labelledby="tf-pruefung-titel"
      data-testid="tf-pruefung"
    >
      <header className="tf-karte__kopf">
        <h3 id="tf-pruefung-titel">
          {t("planung.terminfindung.pruefung.titel")}
        </h3>
        {eintraege.length > 0 && (
          <span className="tf-marke tf-marke--warn">{eintraege.length}</span>
        )}
      </header>
      <div className="tf-karte__inhalt">
        {eintraege.length === 0 ? (
          <p className="tf-hilfe">{t("planung.terminfindung.pruefung.leer")}</p>
        ) : (
          <ul className="tf-pruefliste">
            {eintraege.map((h) => {
              const abweichend =
                h.person != null &&
                h.sender.toLowerCase() !== h.person.toLowerCase();
              return (
                <li key={h.id} className="tf-pruef">
                  <div className="tf-pruef__kopf">
                    <b>{name(h.person ?? h.sender)}</b>
                    <span>
                      {h.result.quelle
                        ? t(`planung.terminfindung.quelle.${h.result.quelle}`)
                        : ""}
                      {" · "}
                      {`${wochentag(h.received_at)} ${uhrzeit(h.received_at)}`}
                    </span>
                  </div>
                  {abweichend && (
                    <p className="tf-pruef__warn">
                      {t("planung.terminfindung.pruefung.absender", {
                        sender: h.sender,
                      })}
                    </p>
                  )}
                  <p>
                    {h.result.werte.length === 0
                      ? t("planung.terminfindung.pruefung.nichtsErkannt")
                      : t("planung.terminfindung.pruefung.erkannt", {
                          werte: h.result.werte
                            .map((w) => {
                              const v = poll.vorschlaege.find(
                                (x) => x.nr === w.vorschlag_nr,
                              );
                              return `${v ? vorschlagKurz(v.start, v.ende) : w.vorschlag_nr}: ${t(
                                `planung.terminfindung.wert.${w.wert}`,
                              )}`;
                            })
                            .join("; "),
                        })}
                  </p>
                  <div className="tf-pruef__aktionen">
                    <Button
                      type="button"
                      variant="primary"
                      size="sm"
                      disabled={beschaeftigt || h.result.werte.length === 0}
                      onClick={() => onUebernehmen(h)}
                    >
                      {t("planung.terminfindung.pruefung.uebernehmen")}
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        <p className="tf-hilfe">
          {t("planung.terminfindung.pruefung.hinweis")}
        </p>
      </div>
    </section>
  );
};

/** Empfehlung mit Rangliste; jede Zahl ist erklärt. */
export const Empfehlung: React.FC<{
  poll: PollDefinition;
  raenge: Rang[] | null;
  fehler: boolean;
  offen: number;
  beschaeftigt: boolean;
  onWaehlen: (nr: number) => void;
}> = ({ poll, raenge, fehler, offen, beschaeftigt, onWaehlen }) => {
  const { t } = useTranslation();
  const gewertet = (raenge ?? []).filter((r) => !r.ausgeschlossen);
  const bester = gewertet[0] ?? null;
  const max = Math.max(1, ...gewertet.map((r) => r.punkte));
  const v = (nr: number) => poll.vorschlaege.find((x) => x.nr === nr);
  const eindeutig =
    bester && (gewertet[1]?.punkte ?? -Infinity) < bester.punkte;
  const besterV = bester ? v(bester.nr) : undefined;
  return (
    <section
      className="tf-karte"
      aria-labelledby="tf-empf-titel"
      data-testid="tf-empfehlung"
    >
      <header className="tf-karte__kopf">
        <h3 id="tf-empf-titel">
          {t("planung.terminfindung.empfehlung.titel")}
        </h3>
        {bester && (
          <span
            className={`tf-marke ${eindeutig ? "tf-marke--ok" : "tf-marke--warn"}`}
          >
            {eindeutig
              ? t("planung.terminfindung.empfehlung.eindeutig")
              : t("planung.terminfindung.empfehlung.gleichstand")}
          </span>
        )}
      </header>
      <div className="tf-karte__inhalt tf-empf">
        {fehler ? (
          <p className="tf-hilfe" role="alert">
            {t("planung.terminfindung.empfehlung.fehler")}
          </p>
        ) : !raenge ? (
          <p className="tf-hilfe">
            {t("planung.terminfindung.empfehlung.laedt")}
          </p>
        ) : !bester || !besterV ? (
          <p className="tf-hilfe">
            {t("planung.terminfindung.empfehlung.keiner")}
          </p>
        ) : (
          <>
            <p className="tf-empf__gross">
              {vorschlagKurz(besterV.start, besterV.ende)}
            </p>
            <p className="tf-hinweis">
              {bester.begruendung}
              {offen > 0 &&
                ` ${t("planung.terminfindung.empfehlung.offen", { count: offen })}`}
            </p>
          </>
        )}
        {raenge && raenge.length > 0 && (
          <ol className="tf-rangliste">
            {raenge.map((r) => {
              const vv = v(r.nr);
              if (!vv) return null;
              return (
                <li
                  key={r.nr}
                  className={r.ausgeschlossen ? "tf-rang--aus" : undefined}
                >
                  <div className="tf-rang">
                    <span>{vorschlagKurz(vv.start, vv.ende)}</span>
                    <b>
                      {r.ausgeschlossen
                        ? t("planung.terminfindung.empfehlung.aus")
                        : t("planung.terminfindung.empfehlung.punkte", {
                            n: r.punkte,
                          })}
                    </b>
                  </div>
                  <div className="tf-balken" aria-hidden>
                    <i
                      style={{
                        width: r.ausgeschlossen
                          ? 0
                          : `${Math.round((Math.max(0, r.punkte) / max) * 100)}%`,
                      }}
                    />
                  </div>
                  <small>{r.begruendung}</small>
                  {!r.ausgeschlossen && r.nr !== bester?.nr && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={beschaeftigt}
                      onClick={() => onWaehlen(r.nr)}
                    >
                      {t("planung.terminfindung.empfehlung.diesen")}
                    </Button>
                  )}
                </li>
              );
            })}
          </ol>
        )}
        <details className="tf-erklaerung">
          <summary>{t("planung.terminfindung.empfehlung.wie")}</summary>
          <p>{t("planung.terminfindung.empfehlung.regel")}</p>
        </details>
        {bester && (
          <div className="tf-aktionen">
            <Button
              type="button"
              variant="primary"
              disabled={beschaeftigt}
              onClick={() => onWaehlen(bester.nr)}
              data-testid="tf-uebernehmen"
            >
              {t("planung.terminfindung.empfehlung.uebernehmen")}
            </Button>
          </div>
        )}
      </div>
    </section>
  );
};
