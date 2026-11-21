import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Plan, PlanLink } from "@/bindings";
import { TabList } from "../ui/TabList";
import { listenGruppe, statusMarken, type ListenGruppe } from "./logik";
import { Statusmarke, terminText } from "./Teile";

type Filter = ListenGruppe | "alle";

/** Liste der Planungen: Termin (aus Outlook), Dauer und Status je Zeile. */
export const PlanungListe: React.FC<{
  plaene: Plan[];
  links: ReadonlyMap<string, PlanLink | null>;
  laden: boolean;
  fehler: string | null;
  auswahl: string | null;
  onAuswahl: (id: string) => void;
}> = ({ plaene, links, laden, fehler, auswahl, onAuswahl }) => {
  const { t, i18n } = useTranslation();
  const [filter, setFilter] = useState<Filter>("aktiv");
  const sichtbar = useMemo(
    () =>
      plaene.filter(
        (p) =>
          filter === "alle" ||
          listenGruppe(p, links.get(p.id) ?? null) === filter,
      ),
    [plaene, links, filter],
  );
  const tabs: { id: Filter; label: string }[] = [
    { id: "aktiv", label: t("planung.liste.filter.aktiv") },
    { id: "archiv", label: t("planung.liste.filter.archiv") },
    { id: "alle", label: t("planung.liste.filter.alle") },
  ];
  return (
    <div className="plg-spalte plg-liste" data-testid="planung-liste">
      <div className="plg-kopf">
        <h2>{t("planung.liste.titel")}</h2>
      </div>
      <TabList
        tabs={tabs}
        value={filter}
        onChange={setFilter}
        compact
        ariaLabel={t("planung.liste.filterLabel")}
        className="plg-filter"
      />
      {fehler !== null && (
        <p role="alert" className="plg-meldung plg-meldung--fehler">
          {t("planung.liste.ladenFehler")}
        </p>
      )}
      {laden && fehler === null && (
        <p role="status" className="plg-meldung">
          {t("planung.liste.laden")}
        </p>
      )}
      {!laden && fehler === null && sichtbar.length === 0 && (
        <p className="plg-meldung" data-testid="planung-leer">
          {plaene.length === 0
            ? t("planung.liste.leer")
            : t("planung.liste.leerFilter")}
        </p>
      )}
      <ul className="plg-zeilen" aria-label={t("planung.liste.ariaLabel")}>
        {sichtbar.map((p) => {
          const link = links.get(p.id) ?? null;
          const termin = terminText(link, i18n.language);
          const gewaehlt = auswahl === p.id;
          return (
            <li key={p.id}>
              <button
                type="button"
                className="plg-zeile"
                aria-current={gewaehlt ? "true" : undefined}
                data-testid="planung-zeile"
                onClick={() => onAuswahl(p.id)}
              >
                <span className="plg-zeile__titel">
                  {p.title.trim() || t("planung.liste.ohneTitel")}
                </span>
                <span className="plg-zeile__meta">
                  {termin || t("planung.liste.keinTermin")}
                  {" · "}
                  {t("planung.liste.minuten", { count: p.duration_min })}
                </span>
                <span className="plg-zeile__status">
                  {statusMarken(p, link).map((m) => (
                    <Statusmarke key={m.schluessel} marke={m} />
                  ))}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
};
