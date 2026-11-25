import React, { useId, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { SearchBox, Skeleton, SkeletonItem } from "@fluentui/react-components";
import {
  Calendar,
  CalendarPlus,
  FolderOpen,
  SearchX,
  Video,
  X,
  type LucideIcon,
} from "lucide-react";
import type { Plan, PlanLink } from "@/bindings";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import { TabList } from "../ui/TabList";
import { trefferTeile } from "../ui/auswahlSuche";
import {
  listenStand,
  titelTreffer,
  zeilenDatum,
  type ListenFilter,
  type ListenZeile,
} from "./liste";
import { Statusmarke } from "./Teile";

export type { ListenFilter } from "./liste";

const SKELETT_ZEILEN = [72, 64, 58, 52, 46];

/** Leerzustand der Liste: Symbol, Satz, höchstens eine Aktion. */
const Leer: React.FC<{
  art: "keine" | "aktiv" | "archiv" | "suche";
  symbol: LucideIcon;
  titel: string;
  text: string;
  aktion?: { label: string; onClick: () => void };
}> = ({ art, symbol: Symbol, titel, text, aktion }) => (
  <div className="plg-leerzustand" data-testid="planung-leer" data-art={art}>
    <Symbol size={20} aria-hidden="true" className="plg-leerzustand__symbol" />
    <p className="plg-leerzustand__text">
      <strong>{titel}</strong> {text}
    </p>
    {aktion && (
      <div className="plg-leerzustand__aktion">
        <Button type="button" size="sm" onClick={aktion.onClick}>
          {aktion.label}
        </Button>
      </div>
    )}
  </div>
);

const Zeile: React.FC<{
  zeile: ListenZeile;
  gewaehlt: boolean;
  suche: string;
  jetzt: number;
  onAuswahl: (id: string) => void;
}> = ({ zeile, gewaehlt, suche, jetzt, onAuswahl }) => {
  const { t, i18n } = useTranslation();
  const { plan, link, marke, online } = zeile;
  const titel = plan.title.trim();
  const beginn = link?.starts_at;
  return (
    <li>
      <button
        type="button"
        className="plg-zeile"
        aria-current={gewaehlt ? "true" : undefined}
        data-testid="planung-zeile"
        onClick={() => onAuswahl(plan.id)}
      >
        <span className="plg-zeile__titel">
          {titel
            ? trefferTeile(titel, titelTreffer(titel, suche)).map((teil, i) =>
                teil.treffer ? (
                  <mark key={i} className="plg-treffer">
                    {teil.text}
                  </mark>
                ) : (
                  <React.Fragment key={i}>{teil.text}</React.Fragment>
                ),
              )
            : t("planung.liste.ohneTitel")}
        </span>
        <span className="plg-zeile__meta">
          <span className="plg-zeile__angabe">
            <Calendar size={12} aria-hidden="true" />
            {beginn != null
              ? zeilenDatum(beginn, i18n.language, jetzt)
              : t("planung.liste.keinTermin")}
          </span>
          {online && (
            <span
              className="plg-zeile__angabe"
              title={t("planung.liste.online")}
            >
              <Video size={12} aria-hidden="true" />
              <span className="sr-only">{t("planung.liste.online")}</span>
            </span>
          )}
          <span className="plg-zeile__angabe">
            {t("planung.liste.minuten", { count: plan.duration_min })}
          </span>
        </span>
        <span className="plg-zeile__rechts">
          <Statusmarke marke={marke} className="plg-zeile__chip" />
        </span>
      </button>
    </li>
  );
};

/**
 * Liste der Planungen (lva#844 U2): Suche, Filter mit Zählern, Zeitgruppen und ein abgeleiteter
 * Zustandschip je Zeile. Filter und Suche hält die Seite, damit sie den Wechsel zwischen Spalte
 * und Schublade überstehen.
 */
export const PlanungListe: React.FC<{
  plaene: Plan[];
  links: ReadonlyMap<string, PlanLink | null>;
  laden: boolean;
  fehler: string | null;
  auswahl: string | null;
  onAuswahl: (id: string) => void;
  filter: ListenFilter;
  onFilter: (filter: ListenFilter) => void;
  suche: string;
  onSuche: (suche: string) => void;
  onNeuLaden: () => void;
  /** Gesetzt, wenn die Liste als Schublade offen ist: Knopf „Planungen schließen“. */
  onSchliessen?: () => void;
  sucheRef?: React.Ref<HTMLInputElement>;
}> = ({
  plaene,
  links,
  laden,
  fehler,
  auswahl,
  onAuswahl,
  filter,
  onFilter,
  suche,
  onSuche,
  onNeuLaden,
  onSchliessen,
  sucheRef,
}) => {
  const { t } = useTranslation();
  const titelId = useId();
  // Die Gruppen beziehen sich auf den Zeitpunkt der letzten Ladung.
  const jetzt = useMemo(() => Date.now(), [plaene]);
  const stand = useMemo(
    () => listenStand(plaene, links, filter, suche, jetzt),
    [plaene, links, filter, suche, jetzt],
  );
  // Nach einem Ladefehler bleibt der letzte Stand lesbar; Leerzustände nur ohne Fehler.
  const geladen = !laden;
  const bereit = geladen && fehler === null;
  const tabs = (["aktiv", "archiv", "alle"] as const).map((id) => ({
    id,
    label: t(`planung.liste.filter.${id}`),
    zaehler: geladen ? stand.zaehler[id] : undefined,
  }));

  let leer: React.ReactNode = null;
  if (bereit && stand.zeilen.length === 0) {
    const z = "planung.liste.leerZustand";
    if (plaene.length === 0)
      leer = (
        <Leer
          art="keine"
          symbol={CalendarPlus}
          titel={t(`${z}.keineTitel`)}
          text={t(`${z}.keineText`)}
        />
      );
    else if (suche.trim() !== "")
      leer = (
        <Leer
          art="suche"
          symbol={SearchX}
          titel={t(`${z}.sucheTitel`, { suche: suche.trim() })}
          text={t(`${z}.sucheText`)}
          aktion={{
            label: t(`${z}.sucheLoeschen`),
            onClick: () => onSuche(""),
          }}
        />
      );
    else if (filter === "archiv")
      leer = (
        <Leer
          art="archiv"
          symbol={FolderOpen}
          titel={t(`${z}.archivTitel`)}
          text={t(`${z}.archivText`)}
          aktion={{
            label: t(`${z}.aktiveAnzeigen`),
            onClick: () => onFilter("aktiv"),
          }}
        />
      );
    else
      leer = (
        <Leer
          art="aktiv"
          symbol={Calendar}
          titel={t(`${z}.aktivTitel`)}
          text={t(`${z}.aktivText`)}
          aktion={{
            label: t(`${z}.archivAnzeigen`),
            onClick: () => onFilter("archiv"),
          }}
        />
      );
  }

  return (
    <section
      className="plg-spalte plg-liste"
      data-testid="planung-liste"
      aria-labelledby={titelId}
      aria-busy={laden ? "true" : undefined}
    >
      <div className="plg-liste__kopf">
        <div className="plg-liste__titelzeile">
          <h2 id={titelId} className="plg-liste__titel">
            {t("planung.liste.titel")}{" "}
            {geladen && stand.aktivGesamt > 0 && (
              <span className="plg-liste__zahl">
                {t("planung.liste.anzahlAktiv", { count: stand.aktivGesamt })}
              </span>
            )}
          </h2>
          {onSchliessen && (
            <IconButton
              icon={<X size={16} aria-hidden="true" />}
              label={t("planung.schublade.schliessen")}
              onClick={onSchliessen}
              data-testid="planung-schublade-zu"
            />
          )}
        </div>
        <SearchBox
          ref={sucheRef}
          className="plg-liste__suche"
          value={suche}
          onChange={(_e, data) => onSuche(data.value)}
          placeholder={t("planung.liste.suche.platzhalter")}
          aria-label={t("planung.liste.suche.label")}
          data-testid="planung-suche"
        />
      </div>
      <TabList
        tabs={tabs}
        value={filter}
        onChange={onFilter}
        compact
        ariaLabel={t("planung.liste.filterLabel")}
        className="plg-filter"
      />
      {fehler !== null && (
        <div className="plg-leerzustand plg-leerzustand--fehler">
          <p role="alert" className="plg-leerzustand__text">
            {t("planung.liste.ladenFehler")}
          </p>
          <div className="plg-leerzustand__aktion">
            <Button
              type="button"
              size="sm"
              onClick={onNeuLaden}
              data-testid="planung-liste-erneut"
            >
              {t("planung.liste.erneut")}
            </Button>
          </div>
        </div>
      )}
      {laden && fehler === null && (
        <div className="plg-skelett" data-testid="planung-liste-laden">
          <Skeleton aria-hidden="true">
            {SKELETT_ZEILEN.map((breite) => (
              <div key={breite} className="plg-skelett__zeile">
                <SkeletonItem size={12} style={{ width: `${breite}%` }} />
                <SkeletonItem size={12} style={{ width: "42%" }} />
              </div>
            ))}
          </Skeleton>
          <p role="status" className="sr-only">
            {t("planung.liste.laden")}
          </p>
        </div>
      )}
      {leer}
      {geladen && stand.gruppen.length > 0 && (
        <div className="plg-gruppen">
          {stand.gruppen.map((g, i) => {
            const kopfId = `${titelId}-g${i}`;
            return (
              <div key={`${g.gruppe}-${i}`} className="plg-gruppe">
                <h3
                  id={kopfId}
                  className="plg-gruppe__titel"
                  data-testid="planung-gruppe"
                >
                  {t(`planung.liste.gruppe.${g.gruppe}`)}
                </h3>
                <ul className="plg-zeilen" aria-labelledby={kopfId}>
                  {g.zeilen.map((z) => (
                    <Zeile
                      key={z.plan.id}
                      zeile={z}
                      gewaehlt={auswahl === z.plan.id}
                      suche={suche}
                      jetzt={jetzt}
                      onAuswahl={onAuswahl}
                    />
                  ))}
                </ul>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
};
