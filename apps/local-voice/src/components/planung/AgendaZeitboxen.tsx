import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";

interface AgendaPunkt {
  topic: string;
  minutes: number;
  owner?: string | null;
}

interface Props {
  agenda: readonly AgendaPunkt[];
}

/** Restzeit in ganzen Sekunden; ein angebrochener Überzug ist sofort sichtbar. */
export function zeitboxRestzeit(minuten: number, verbrauchtMs: number): number {
  const dauer = Number.isFinite(minuten) ? Math.max(0, minuten) * 60 : 0;
  const rest = dauer - Math.max(0, verbrauchtMs) / 1000;
  return rest < 0 ? Math.floor(rest) : Math.ceil(rest);
}

function zeitText(sekunden: number): string {
  const betrag = Math.abs(sekunden);
  return `${Math.floor(betrag / 60)}:${String(betrag % 60).padStart(2, "0")}`;
}

/** Ein neuer Agendastand beginnt eine neue, rein lokale Zeitmessung. */
export const AgendaZeitboxen: React.FC<Props> = ({ agenda }) => (
  <Zeitboxen key={JSON.stringify(agenda)} agenda={agenda} />
);

const Zeitboxen: React.FC<Props> = ({ agenda }) => {
  const { t } = useTranslation();
  const [punkt, setPunkt] = useState(0);
  const [verbrauch, setVerbrauch] = useState<number[]>(() => agenda.map(() => 0));
  const [seit, setSeit] = useState<number | null>(null);
  const [jetzt, setJetzt] = useState(() => Date.now());

  useEffect(() => {
    if (seit === null) return;
    const timer = setInterval(() => setJetzt(Date.now()), 250);
    return () => clearInterval(timer);
  }, [seit]);

  const wechseln = (naechster: number) => {
    if (naechster < 0 || naechster >= agenda.length || naechster === punkt) return;
    const zeit = Date.now();
    if (seit !== null) {
      setVerbrauch((bisher) =>
        bisher.map((ms, index) =>
          index === punkt ? ms + Math.max(0, zeit - seit) : ms,
        ),
      );
      setSeit(zeit);
    }
    setJetzt(zeit);
    setPunkt(naechster);
  };

  return (
    <section className="plg-karte" data-testid="planung-zeitboxen">
      <h3 className="plg-karte__titel">{t("planung.zeitboxen.titel")}</h3>
      {agenda.length === 0 ? (
        <p className="plg-meldung">{t("planung.zeitboxen.leer")}</p>
      ) : (
        <>
          <ol className="space-y-2">
            {agenda.map((a, index) => {
              const aktiv = index === punkt;
              const vergangen =
                (verbrauch[index] ?? 0) +
                (aktiv && seit !== null ? Math.max(0, jetzt - seit) : 0);
              const rest = zeitboxRestzeit(a.minutes, vergangen);
              const ueberzogen = vergangen > Math.max(0, a.minutes) * 60_000;
              return (
                <li
                  key={index}
                  aria-current={aktiv ? "step" : undefined}
                  data-testid="planung-zeitbox"
                  data-aktiv={aktiv}
                  data-ueberzug={ueberzogen}
                  className={aktiv ? "font-semibold" : ""}
                >
                  <span>{a.topic}</span>
                  {a.owner && <span className="text-text/70"> · {a.owner}</span>}
                  <p className={ueberzogen ? "text-red-500" : "text-text/70"}>
                    {t(
                      ueberzogen
                        ? "planung.zeitboxen.ueberzug"
                        : "planung.zeitboxen.restzeit",
                      { zeit: zeitText(rest) },
                    )}
                  </p>
                </li>
              );
            })}
          </ol>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              disabled={seit !== null}
              data-testid="planung-zeitbox-start"
              onClick={() => {
                const zeit = Date.now();
                setJetzt(zeit);
                setSeit(zeit);
              }}
            >
              {t("planung.zeitboxen.start")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              disabled={punkt === 0}
              data-testid="planung-zeitbox-zurueck"
              onClick={() => wechseln(punkt - 1)}
            >
              {t("planung.zeitboxen.zurueck")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              disabled={punkt === agenda.length - 1}
              data-testid="planung-zeitbox-weiter"
              onClick={() => wechseln(punkt + 1)}
            >
              {t("planung.zeitboxen.weiter")}
            </Button>
          </div>
        </>
      )}
    </section>
  );
};
