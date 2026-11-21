import React from "react";
import { useTranslation } from "react-i18next";
import { ALLES_ID, VORLAGEN } from "@/components/voice-call/assistenten";
import type { Profil } from "@/components/voice-call/assistenten";
import type { Assistenten } from "@/components/voice-call/profil/speicher";
import type { Eintrag } from "@/stores/assistentEditStore";
import { AssistentMenue } from "./AssistentMenue";

interface Props {
  assistenten: Assistenten;
  offen: Eintrag | null;
  /** Der gewählte Assistent (Auswahl im Gespräch). */
  gewaehltId: string;
  /** Id des eigenen Assistenten mit ungespeicherten Änderungen. */
  entwurfId: string | null;
  /** Schreibende Vorgänge erlaubt (Speicher lesbar, kein Vorgang läuft). */
  schreibt: boolean;
  onOeffne: (e: Eintrag) => void;
  onKopieren: (p: Profil) => void;
  onUmbenennen: (p: Profil) => void;
  onExportieren: (p: Profil) => void;
  onLoeschen: (p: Profil) => void;
}

/**
 * Die Liste der Assistenten: Vorlagen (nur ansehen und kopieren), eigene Assistenten und, wenn der Speicher Dateien
 * abgewiesen hat, diese als „Ungültige Dateien“. Jede Zeile ist ein Knopf (Tastatur: Tab, Eingabe) mit dem Namen und, wenn der
 * Assistent gewählt ist oder ungespeicherte Änderungen hat, einer Marke; das Menü daneben hat die Aktionen.
 */
export const AssistentenListe: React.FC<Props> = ({
  assistenten,
  offen,
  gewaehltId,
  entwurfId,
  schreibt,
  onOeffne,
  onKopieren,
  onUmbenennen,
  onExportieren,
  onLoeschen,
}) => {
  const { t } = useTranslation();
  const vorlagen = VORLAGEN.filter((p) => p.id !== ALLES_ID);
  const istOffen = (art: Eintrag["art"], id: string) =>
    offen?.art === art &&
    (offen.art === "abgewiesen" ? offen.datei === id : offen.id === id);

  const zeile = (
    p: Profil,
    art: "vorlage" | "eigen",
    kopf: React.ReactNode,
  ) => (
    <li
      key={p.id}
      className="lv-asst-zeile"
      data-offen={istOffen(art, p.id) ? "true" : "false"}
      data-testid={`assistent-zeile-${p.id}`}
    >
      <button
        type="button"
        className="lv-asst-zeile__haupt"
        aria-current={istOffen(art, p.id) ? "true" : undefined}
        onClick={() => onOeffne({ art, id: p.id })}
        data-testid={`assistent-oeffnen-${p.id}`}
      >
        <span className="lv-asst-zeile__kopf">
          <span className="lv-asst-zeile__name">{p.name}</span>
          {kopf}
        </span>
        {p.beschreibung && (
          <span className="lv-asst-zeile__beschreibung">{p.beschreibung}</span>
        )}
      </button>
      <AssistentMenue
        name={p.name}
        art={art}
        schreibt={schreibt}
        onKopieren={() => onKopieren(p)}
        onExportieren={() => onExportieren(p)}
        onUmbenennen={art === "eigen" ? () => onUmbenennen(p) : undefined}
        onLoeschen={art === "eigen" ? () => onLoeschen(p) : undefined}
        testId={`assistent-menue-${p.id}`}
      />
    </li>
  );

  const marken = (p: Profil) => (
    <>
      {p.id === gewaehltId && (
        <span className="lv-asst-marke" data-testid="assistent-marke-gewaehlt">
          {t("assistenten.liste.gewaehlt")}
        </span>
      )}
      {p.id === entwurfId && (
        <span
          className="lv-asst-marke"
          data-art="warn"
          data-testid="assistent-marke-entwurf"
        >
          {t("assistenten.liste.ungespeichert")}
        </span>
      )}
    </>
  );

  return (
    <nav
      aria-label={t("assistenten.liste.label")}
      data-testid="assistenten-liste"
    >
      <section className="lv-asst-gruppe" aria-labelledby="asst-g-eigene">
        <h2 className="lv-asst-gruppe__titel" id="asst-g-eigene">
          {t("assistenten.liste.eigene")}
        </h2>
        {assistenten.eigene.length === 0 ? (
          <p className="lv-asst-hinweis px-2.5" data-testid="assistenten-leer">
            {t("assistenten.liste.keine")}
          </p>
        ) : (
          <ul>
            {assistenten.eigene.map((e) =>
              zeile(e.profil, "eigen", marken(e.profil)),
            )}
          </ul>
        )}
      </section>
      <section className="lv-asst-gruppe" aria-labelledby="asst-g-vorlagen">
        <h2 className="lv-asst-gruppe__titel" id="asst-g-vorlagen">
          {t("assistenten.liste.vorlagen")}
        </h2>
        <ul>
          {vorlagen.map((p) =>
            zeile(
              p,
              "vorlage",
              p.id === gewaehltId ? (
                <span className="lv-asst-marke">
                  {t("assistenten.liste.gewaehlt")}
                </span>
              ) : null,
            ),
          )}
        </ul>
      </section>
      {assistenten.abgewiesen.length > 0 && (
        <section className="lv-asst-gruppe" aria-labelledby="asst-g-abgewiesen">
          <h2 className="lv-asst-gruppe__titel" id="asst-g-abgewiesen">
            {t("assistenten.liste.abgewiesen")}
          </h2>
          <ul>
            {assistenten.abgewiesen.map((x) => (
              <li
                key={x.datei}
                className="lv-asst-zeile"
                data-offen={istOffen("abgewiesen", x.datei) ? "true" : "false"}
                data-testid={`assistent-abgewiesen-${x.datei}`}
              >
                <button
                  type="button"
                  className="lv-asst-zeile__haupt"
                  aria-current={
                    istOffen("abgewiesen", x.datei) ? "true" : undefined
                  }
                  onClick={() =>
                    onOeffne({ art: "abgewiesen", datei: x.datei })
                  }
                >
                  <span className="lv-asst-zeile__kopf">
                    <span className="lv-asst-zeile__name">{x.datei}</span>
                    <span className="lv-asst-marke" data-art="warn">
                      {t("assistenten.liste.ungueltig")}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </nav>
  );
};
