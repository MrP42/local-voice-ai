import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Einladung } from "@/bindings";
import { TabList } from "../ui/TabList";

type Ansicht = "formatiert" | "text";

/**
 * Vorschau des Einladungstexts, wie ihn das Backend erzeugt (Block der App ohne Zeit, Ort und
 * Teilnehmende; die führt Outlook). Das HTML liegt in einem Rahmen ohne Rechte (`sandbox=""`):
 * keine Skripte, keine Formulare, keine Navigation.
 */
export const EinladungsVorschau: React.FC<{
  einladung: Einladung | null;
  fehler: boolean;
}> = ({ einladung, fehler }) => {
  const { t } = useTranslation();
  const [ansicht, setAnsicht] = useState<Ansicht>("formatiert");
  return (
    <section
      className="plg-karte"
      aria-labelledby="plg-vorschau-titel"
      data-testid="planung-vorschau"
    >
      <h2 id="plg-vorschau-titel" className="plg-karte__titel">
        {t("planung.vorschau.titel")}
      </h2>
      <p className="plg-karte__hilfe">{t("planung.vorschau.hilfe")}</p>
      <TabList
        tabs={[
          { id: "formatiert", label: t("planung.vorschau.formatiert") },
          { id: "text", label: t("planung.vorschau.text") },
        ]}
        value={ansicht}
        onChange={setAnsicht}
        compact
        ariaLabel={t("planung.vorschau.ansicht")}
      />
      {fehler && (
        <p role="alert" className="plg-meldung plg-meldung--fehler">
          {t("planung.vorschau.fehler")}
        </p>
      )}
      {!fehler && !einladung && (
        <p role="status" className="plg-meldung">
          {t("planung.vorschau.laden")}
        </p>
      )}
      {einladung && ansicht === "formatiert" && (
        <iframe
          title={t("planung.vorschau.rahmen")}
          sandbox=""
          srcDoc={einladung.html}
          className="plg-vorschau__rahmen"
          data-testid="planung-vorschau-html"
        />
      )}
      {einladung && ansicht === "text" && (
        <pre className="plg-vorschau__text" data-testid="planung-vorschau-text">
          {einladung.text}
        </pre>
      )}
    </section>
  );
};
