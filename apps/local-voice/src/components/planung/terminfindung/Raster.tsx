import React from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { PersonAvailability, Vorschlag } from "@/bindings";
import { Button } from "../../ui/Button";
import {
  intervallIndex,
  RASTER_MIN,
  TAG_VON,
  ZEILEN,
  zellenZeit,
  zellStand,
} from "./logik";
import { kalenderwoche, spanne, tagMonat, uhrzeit, wochentag } from "./zeit";

interface Props {
  woche: number;
  onWoche: (woche: number) => void;
  verfuegbar: PersonAvailability[] | null;
  laedt: boolean;
  fehler: string | null;
  pflicht: Set<string>;
  dauerMin: number;
  vorschlaege: Vorschlag[];
  /** `null` = nur ansehen (Umfrage ist versendet). */
  onZelle: ((start: number) => void) | null;
  stand: string | null;
}

const TAGE = [0, 1, 2, 3, 4];
const WOCHE_MS = 7 * 86_400_000;

/**
 * Wochenraster mit Heatmap der internen Verfügbarkeit (nur frei/belegt aus Outlook, keine
 * Termininhalte). Ein Klick auf eine Zelle setzt oder entfernt einen Vorschlag ab dieser Zeit.
 */
export const Raster: React.FC<Props> = ({
  woche,
  onWoche,
  verfuegbar,
  laedt,
  fehler,
  pflicht,
  dauerMin,
  vorschlaege,
  onZelle,
  stand,
}) => {
  const { t } = useTranslation();
  const dauerIntervalle = Math.max(1, Math.ceil(dauerMin / RASTER_MIN));
  const vorschlagAb = new Map(vorschlaege.map((v) => [v.start, v]));
  const zeilenJeVorschlag = Math.max(1, Math.round(dauerMin / RASTER_MIN));
  // Zellen, die von einem längeren Vorschlag überdeckt werden, zeigen nur Farbe.
  const ueberdeckt = new Set<number>();
  for (const v of vorschlaege) {
    for (let i = 1; i < zeilenJeVorschlag; i++)
      ueberdeckt.add(v.start + i * RASTER_MIN * 60_000);
  }

  return (
    <section
      className="tf-karte tf-raster"
      aria-labelledby="tf-raster-titel"
      data-testid="tf-raster"
    >
      <header className="tf-karte__kopf">
        <h3 id="tf-raster-titel">
          {t("planung.terminfindung.raster.titel", {
            kw: kalenderwoche(woche),
          })}
        </h3>
        {laedt ? (
          <span className="tf-marke">
            {t("planung.terminfindung.raster.laedt")}
          </span>
        ) : fehler ? (
          <span className="tf-marke tf-marke--warn" role="status">
            {t("planung.terminfindung.raster.unbekannt")}
          </span>
        ) : stand ? (
          <span className="tf-marke tf-marke--ok">
            {t("planung.terminfindung.raster.stand", { zeit: stand })}
          </span>
        ) : null}
        <div className="tf-karte__aktionen">
          <Button
            type="button"
            variant="ghost"
            icon={<ChevronLeft size={16} />}
            aria-label={t("planung.terminfindung.raster.zurueck")}
            title={t("planung.terminfindung.raster.zurueck")}
            onClick={() => onWoche(woche - WOCHE_MS)}
          />
          <Button
            type="button"
            variant="ghost"
            icon={<ChevronRight size={16} />}
            aria-label={t("planung.terminfindung.raster.weiter")}
            title={t("planung.terminfindung.raster.weiter")}
            onClick={() => onWoche(woche + WOCHE_MS)}
          />
        </div>
      </header>
      <div
        className="tf-woche"
        role="grid"
        aria-label={t("planung.terminfindung.raster.aria")}
      >
        <div role="row" className="tf-woche__zeile tf-woche__zeile--kopf">
          <span role="columnheader" />
          {TAGE.map((tag) => {
            const ms = zellenZeit(woche, tag, 0);
            return (
              <span role="columnheader" key={tag} className="tf-woche__tag">
                {wochentag(ms)}
                <small>{tagMonat(ms)}</small>
              </span>
            );
          })}
        </div>
        {Array.from({ length: ZEILEN }, (_, zeile) => (
          <div role="row" className="tf-woche__zeile" key={zeile}>
            <span role="rowheader" className="tf-woche__zeit">
              {zeile % 2 === 0
                ? `${String(TAG_VON + zeile / 2).padStart(2, "0")}:00`
                : ""}
            </span>
            {TAGE.map((tag) => {
              const start = zellenZeit(woche, tag, zeile);
              const z = verfuegbar
                ? zellStand(
                    verfuegbar,
                    pflicht,
                    intervallIndex(woche, tag, zeile),
                    dauerIntervalle,
                  )
                : null;
              const v = vorschlagAb.get(start);
              const vergangen = start < Date.now();
              const beschreibung = [
                `${wochentag(start)} ${tagMonat(start)} ${uhrzeit(start)}`,
                z && z.bekannt > 0
                  ? t("planung.terminfindung.raster.zelle", {
                      frei: z.frei,
                      von: z.bekannt,
                    })
                  : t("planung.terminfindung.raster.zelleUnbekannt"),
                z && z.bekannt > 0 && z.unbekannt > 0
                  ? t("planung.terminfindung.raster.ohneKalender", {
                      count: z.unbekannt,
                    })
                  : "",
                v
                  ? t("planung.terminfindung.raster.istVorschlag", { nr: v.nr })
                  : "",
              ]
                .filter(Boolean)
                .join(", ");
              return (
                <button
                  type="button"
                  role="gridcell"
                  key={tag}
                  className={`tf-zelle tf-zelle--h${z?.heat ?? 0}${ueberdeckt.has(start) ? " tf-zelle--belegt" : ""}`}
                  aria-label={beschreibung}
                  aria-selected={!!v}
                  title={beschreibung}
                  disabled={!onZelle || vergangen}
                  onClick={() => onZelle?.(start)}
                  data-testid={v ? "tf-vorschlag" : undefined}
                >
                  {v && (
                    <span
                      className="tf-vorschlag"
                      style={{
                        height: `calc(${zeilenJeVorschlag * 100}% + ${zeilenJeVorschlag - 1}px - 4px)`,
                      }}
                    >
                      <b>
                        {v.nr}
                        <span className="tf-vorschlag__zeit">
                          {` · ${spanne(v.start, v.ende)}`}
                        </span>
                      </b>
                      {z && z.bekannt > 0 && (
                        <small>
                          {z.frei === z.bekannt
                            ? t("planung.terminfindung.raster.alleFrei", {
                                n: z.bekannt,
                              })
                            : t("planung.terminfindung.raster.teilFrei", {
                                frei: z.frei,
                                von: z.bekannt,
                              })}
                        </small>
                      )}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <footer className="tf-legende">
        <span>
          <i className="tf-zelle--h5" />
          {t("planung.terminfindung.legende.alle")}
        </span>
        <span>
          <i className="tf-zelle--h3" />
          {t("planung.terminfindung.legende.meiste")}
        </span>
        <span>
          <i className="tf-zelle--h2" />
          {t("planung.terminfindung.legende.teil")}
        </span>
        <span>
          <i className="tf-zelle--h1" />
          {t("planung.terminfindung.legende.pflicht")}
        </span>
        <span>
          <i className="tf-zelle--h0" />
          {t("planung.terminfindung.legende.unbekannt")}
        </span>
        <span className="tf-legende__hinweis">
          {t("planung.terminfindung.legende.datenschutz")}
        </span>
      </footer>
    </section>
  );
};
