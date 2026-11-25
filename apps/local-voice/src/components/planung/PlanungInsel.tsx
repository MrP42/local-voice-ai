import React, { useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Tab, TabList, Tooltip } from "@fluentui/react-components";
import { Check, Radio, TriangleAlert, type LucideIcon } from "lucide-react";
import type { StatusMarke } from "./logik";
import {
  REGISTER,
  type LaufStand,
  type RegisterId,
  type RegisterInfo,
} from "./register";
import { Statusmarke } from "./Teile";

export interface InselFakt {
  id: string;
  icon: LucideIcon;
  text: string;
}

interface Props {
  titel: string;
  marken: readonly StatusMarke[];
  lauf: LaufStand | null;
  fakten: readonly InselFakt[];
  /** Befehle des aktiven Registers (höchstens einer primär) und das „…“-Menü. */
  aktionen: React.ReactNode;
  register: RegisterId;
  infos: Record<RegisterId, RegisterInfo>;
  /** Register, die (noch) nicht wählbar sind, z. B. vor dem ersten Speichern. */
  gesperrt: ReadonlySet<RegisterId>;
  onRegister: (id: RegisterId) => void;
  /** Klick auf die Marke „Verschoben“: springt zur Meldung unter „Termin & Zusagen“. */
  onVerschoben: () => void;
}

/** Wahr, solange die Insel beim Scrollen verdichtet ist (Befehle zeigen dann nur ihr Symbol). */
const InselKompakt = React.createContext(false);

/**
 * Ein Befehl der Insel: in der verdichteten Insel zeigt er nur sein Symbol und bekommt darum einen
 * Fluent-Tooltip mit seinem Text; sonst steht der Text sichtbar am Knopf.
 */
export const InselBefehl: React.FC<{
  text: string;
  children: React.ReactElement;
}> = ({ text, children }) =>
  useContext(InselKompakt) ? (
    <Tooltip content={text} relationship="label" positioning="below">
      {children}
    </Tooltip>
  ) : (
    children
  );

/** Id des Registers (`aria-labelledby` des Panels). */
export const registerTabId = (id: RegisterId) => `plg-register-${id}`;
/** Id des Panels (`aria-controls` des Registers). */
export const registerPanelId = (id: RegisterId) => `plg-register-panel-${id}`;

const STAND_SYMBOL: Partial<Record<RegisterInfo["stand"], LucideIcon>> = {
  fertig: Check,
  achtung: TriangleAlert,
  laeuft: Radio,
};

/**
 * Kopf der Planung („Insel“, lva#844 U1): Titel, Zustandsmarken, Eckdaten, die Befehle des
 * aktiven Registers und die Phasenleiste als Fluent-TabList. Abschaltbar über die Klasse
 * `plg-insel--aus` (normaler Fluent-Kopf); beim Scrollen eine Zeile.
 */
export const PlanungInsel: React.FC<Props> = ({
  titel,
  marken,
  lauf,
  fakten,
  aktionen,
  register,
  infos,
  gesperrt,
  onRegister,
  onVerschoben,
}) => {
  const { t } = useTranslation();
  const listeRef = useRef<HTMLDivElement>(null);
  const obenRef = useRef<HTMLDivElement>(null);
  const tiefRef = useRef<HTMLDivElement>(null);
  const [kompakt, setKompakt] = useState(false);

  // Zwei Marken im scrollenden Bereich: verdichten, sobald die tiefe Marke (24 px) hinaus
  // ist (Lage in planung.css); wieder aufklappen erst, wenn die obere Marke (0 px) zurück ist. Die Lücke dazwischen
  // verhindert ein Pendeln, wenn das Verdichten die Seite kürzer macht.
  useEffect(() => {
    if (typeof IntersectionObserver === "undefined") return;
    const oben = obenRef.current;
    const tief = tiefRef.current;
    if (!oben || !tief) return;
    const beobachter = new IntersectionObserver((eintraege) => {
      for (const e of eintraege) {
        if (e.target === tief && !e.isIntersecting) setKompakt(true);
        if (e.target === oben && e.isIntersecting) setKompakt(false);
      }
    });
    beobachter.observe(oben);
    beobachter.observe(tief);
    return () => beobachter.disconnect();
  }, []);

  const waehlbar = REGISTER.filter((id) => !gesperrt.has(id));

  const onKeyDown = (event: React.KeyboardEvent, id: RegisterId) => {
    const tasten = ["ArrowRight", "ArrowLeft", "Home", "End"];
    if (!tasten.includes(event.key)) return;
    event.preventDefault();
    const rtl =
      event.currentTarget.closest("[dir]")?.getAttribute("dir") === "rtl";
    const index = waehlbar.indexOf(id);
    const schritt = (event.key === "ArrowRight" ? 1 : -1) * (rtl ? -1 : 1);
    const naechstes =
      event.key === "Home"
        ? waehlbar[0]
        : event.key === "End"
          ? waehlbar[waehlbar.length - 1]
          : waehlbar[(index + schritt + waehlbar.length) % waehlbar.length];
    onRegister(naechstes);
    // Der Fokus folgt der Auswahl.
    listeRef.current
      ?.querySelector<HTMLElement>(`#${registerTabId(naechstes)}`)
      ?.focus();
  };

  return (
    <>
      <div ref={obenRef} className="plg-insel__marke" aria-hidden="true" />
      <div
        ref={tiefRef}
        className="plg-insel__marke plg-insel__marke--tief"
        aria-hidden="true"
      />
      <header
        className={`plg-insel${kompakt ? " plg-insel--kompakt" : ""}`}
        data-testid="planung-insel"
        data-kompakt={kompakt ? "true" : "false"}
      >
        <div className="plg-insel__oben">
          <div className="plg-insel__text">
            <h2 className="plg-insel__titel">
              <span className="plg-insel__name" title={titel}>
                {titel}
              </span>
              {(marken.length > 0 || lauf) && (
                <span className="plg-insel__chips">
                  {marken.map((m) =>
                    m.schluessel === "verschoben" ? (
                      <button
                        key={m.schluessel}
                        type="button"
                        className="plg-insel__chipknopf"
                        onClick={onVerschoben}
                        title={t("planung.insel.verschobenHilfe")}
                        data-testid="planung-insel-verschoben"
                      >
                        <Statusmarke
                          marke={m}
                          testId="planung-insel-status-verschoben"
                          className={`plg-chip plg-chip--${m.ton}`}
                        />
                      </button>
                    ) : (
                      <Statusmarke
                        key={m.schluessel}
                        marke={m}
                        testId={`planung-insel-status-${m.schluessel}`}
                        className={`plg-chip plg-chip--${m.ton} plg-chip--still`}
                      />
                    ),
                  )}
                  {lauf && (
                    <span
                      className="plg-insel__lauf"
                      data-testid="planung-insel-lauf"
                    >
                      <span
                        className="plg-insel__lauf-punkt"
                        aria-hidden="true"
                      />
                      {t(
                        {
                          transkription: "planung.insel.laeuftTranskription",
                          notizen: "planung.insel.laeuftNotizen",
                          nicht_pruefbar: "planung.insel.laeuftNichtPruefbar",
                        }[lauf.art],
                      )}
                    </span>
                  )}
                </span>
              )}
            </h2>
            {fakten.length > 0 && (
              <ul
                className="plg-insel__fakten"
                aria-label={t("planung.insel.fakten")}
              >
                {fakten.map(({ id, icon: Symbol, text }) => (
                  <li key={id} data-fakt={id}>
                    <Symbol size={14} aria-hidden="true" />
                    {text}
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="plg-insel__aktionen">
            <InselKompakt.Provider value={kompakt}>
              {aktionen}
            </InselKompakt.Provider>
          </div>
        </div>
        <TabList
          ref={listeRef}
          className="plg-phasen"
          reserveSelectedTabSpace={false}
          aria-label={t("planung.register.label")}
          selectedValue={register}
          onTabSelect={(_e, data) => onRegister(data.value as RegisterId)}
        >
          {REGISTER.map((id, index) => {
            const info = infos[id];
            const Symbol = STAND_SYMBOL[info.stand];
            const aus = gesperrt.has(id);
            return (
              <Tab
                key={id}
                value={id}
                id={registerTabId(id)}
                aria-controls={registerPanelId(id)}
                disabled={aus}
                className="plg-phase"
                data-stand={info.stand}
                data-testid={`planung-register-${id}`}
                onKeyDown={(e) => onKeyDown(e, id)}
                onFocus={() => {
                  if (id !== register) onRegister(id);
                }}
              >
                <span className="plg-phase__punkt" aria-hidden="true">
                  {info.stand === "fertig" ? (
                    <Check size={14} aria-hidden="true" />
                  ) : (
                    index + 1
                  )}
                </span>
                <span className="plg-phase__text">
                  <span className="plg-phase__name">
                    {t(`planung.register.${id}`)}
                  </span>
                  <span
                    className="plg-phase__info"
                    data-testid={`planung-register-info-${id}`}
                  >
                    {t(`planung.register.info.${info.schluessel}`, {
                      ...info.werte,
                      marke:
                        typeof info.werte?.marke === "string"
                          ? t(`planung.status.${info.werte.marke}`)
                          : undefined,
                    })}
                    {Symbol && info.stand === "achtung" && (
                      <Symbol
                        size={12}
                        aria-hidden="true"
                        className="plg-phase__warn"
                      />
                    )}
                  </span>
                </span>
              </Tab>
            );
          })}
        </TabList>
      </header>
    </>
  );
};
