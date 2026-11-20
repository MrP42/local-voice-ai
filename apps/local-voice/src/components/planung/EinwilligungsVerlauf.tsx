import React from "react";
import { useTranslation } from "react-i18next";
import type { EinwilligungsEintrag } from "@/bindings";
import { Button } from "../ui/Button";
import { UNBEKANNT, zustandsVerlauf } from "./durchfuehren";
import { planungsZeitText } from "./logik";

export const EinwilligungsVerlauf: React.FC<{
  eintraege: EinwilligungsEintrag[] | null;
  fehler: boolean;
  onErneutVersuchen: () => void;
}> = ({ eintraege, fehler, onErneutVersuchen }) => {
  const { t } = useTranslation();
  return (
    <section
      className="plg-karte"
      data-testid="planung-einw-verlauf"
      aria-labelledby="plg-verlauf-titel"
    >
      <h2 id="plg-verlauf-titel" className="plg-karte__titel">
        {t("planung.verlauf.titel")}
      </h2>
      {fehler ? (
        <div className="plg-meldung plg-meldung--fehler" role="alert">
          <p>{t("planung.verlauf.ladeFehler")}</p>
          <Button type="button" onClick={onErneutVersuchen}>
            {t("planung.einwilligungen.erneutVersuchen")}
          </Button>
        </div>
      ) : eintraege === null ? (
        <p className="plg-meldung" role="status">
          {t("planung.verlauf.laden")}
        </p>
      ) : eintraege.length === 0 ? (
        <p className="plg-meldung">{t("planung.verlauf.leer")}</p>
      ) : (
        <ol className="plg-verlauf list-none p-0 space-y-1">
          {zustandsVerlauf(eintraege).map(
            ({ einwilligung: c, festgehalten_am }, index) => (
              <li key={index} data-testid="planung-verlauf-eintrag">
                <span className="plg-person__name">
                  {c.person.trim() === "" || c.person === UNBEKANNT
                    ? t("planung.verlauf.nichtIdentifiziert")
                    : c.person}
                </span>{" "}
                <span className="plg-person__stand">
                  {t(`planung.einwilligungen.zustand.${c.state}`)}
                </span>{" "}
                {c.channel !== null && (
                  <>{t(`planung.einwilligungen.kanal.${c.channel}`)} </>
                )}
                <span>{planungsZeitText(festgehalten_am)}</span>
              </li>
            ),
          )}
        </ol>
      )}
    </section>
  );
};
