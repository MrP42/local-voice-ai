import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Consent, ConsentState, StartModus } from "@/bindings";
import { translateMeetingError } from "@/components/settings/meetings/meetingErrors";
import { Button } from "../ui/Button";
import { api } from "./api";
import {
  anwesendeAusTermin,
  anwesendeFuerBackend,
  anwesenderHinzufuegen,
  personStand,
  startGruende,
  type Anwesender,
} from "./durchfuehren";

export interface StartErgebnis {
  art: "transkription" | "notizen";
  meetingId: string;
  anwesende: string[];
}

interface Props {
  plan: { id: string; title: string; transcription_planned: boolean };
  /** Termin der Planung (Kalender-Cache); null: keine Verknüpfung bekannt. */
  eventKey: string | null;
  /** Teilnehmende des Termins (Startwert der Anwesendenliste). */
  teilnehmende: readonly { email: string | null; name: string | null }[];
  captureSystem?: boolean;
  onGestartet: (e: StartErgebnis) => void;
  onAbbrechen?: () => void;
}

export const StartPanel: React.FC<Props> = ({
  plan,
  eventKey,
  teilnehmende,
  captureSystem = true,
  onGestartet,
  onAbbrechen,
}) => {
  const { t } = useTranslation();
  const titelId = useId();
  const [anwesende, setAnwesende] = useState<Anwesender[]>(() =>
    anwesendeAusTermin(teilnehmende),
  );
  const [einwilligungen, setEinwilligungen] = useState<Consent[] | null>(null);
  const [ladeFehler, setLadeFehler] = useState(false);
  const [modus, setModus] = useState<StartModus | null>(null);
  const [beschaeftigt, setBeschaeftigt] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  const [hinzufuegenText, setHinzufuegenText] = useState("");
  const [hinzufuegenFehler, setHinzufuegenFehler] = useState<string | null>(
    null,
  );
  const gesperrt = useRef(false);

  useEffect(() => {
    let aktiv = true;
    setEinwilligungen(null);
    setLadeFehler(false);
    void api.einwilligungen(plan.id).then(
      (liste) => {
        if (aktiv) setEinwilligungen(liste);
      },
      () => {
        if (aktiv) setLadeFehler(true);
      },
    );
    return () => {
      aktiv = false;
    };
  }, [plan.id]);

  const beginnen = () => {
    if (gesperrt.current) return false;
    gesperrt.current = true;
    setBeschaeftigt(true);
    setFehler(null);
    return true;
  };

  const beenden = () => {
    gesperrt.current = false;
    setBeschaeftigt(false);
  };

  const laden = async () => {
    setLadeFehler(false);
    try {
      setEinwilligungen(await api.einwilligungen(plan.id));
    } catch {
      setLadeFehler(true);
    }
  };

  const erneutLaden = async () => {
    if (!beginnen()) return;
    try {
      await laden();
    } finally {
      beenden();
    }
  };

  const setzen = async (person: string, state: ConsentState) => {
    if (!beginnen()) return;
    setModus(null);
    try {
      await api.einwilligungSetzen({
        plan_id: plan.id,
        person,
        state,
        channel: "muendlich_bei_start",
        recorded_at: Date.now(),
      });
      await laden();
    } catch {
      setFehler(t("planung.einwilligungen.schreibFehler"));
    } finally {
      beenden();
    }
  };

  const hinzufuegen = () => {
    if (gesperrt.current) return;
    const ergebnis = anwesenderHinzufuegen(anwesende, hinzufuegenText);
    if (ergebnis.fehler) {
      setHinzufuegenFehler(
        t(
          ergebnis.fehler === "ungueltig"
            ? "planung.beginnen.hinzuUngueltig"
            : "planung.beginnen.hinzuDoppelt",
        ),
      );
      return;
    }
    setAnwesende(ergebnis.liste);
    setHinzufuegenText("");
    setHinzufuegenFehler(null);
    setModus(null);
  };

  const starten = async (nurNotizen: boolean) => {
    if (!nurNotizen && (!plan.transcription_planned || eventKey === null))
      return;
    if (!beginnen()) return;
    const personen = anwesendeFuerBackend(anwesende);
    setModus(null);
    try {
      if (nurNotizen) {
        const meeting = await api.notizenStarten(
          plan.id,
          plan.title.trim() || t("planung.beginnen.notizTitel"),
        );
        onGestartet({
          art: "notizen",
          meetingId: meeting.id,
          anwesende: personen,
        });
      } else {
        const pruefung = await api.startModus(plan.id, personen);
        setModus(pruefung);
        if (pruefung.modus === "mit_transkription" && eventKey !== null) {
          const meeting = await api.startAusTermin(eventKey, captureSystem);
          onGestartet({
            art: "transkription",
            meetingId: meeting.id,
            anwesende: personen,
          });
        }
      }
    } catch (e) {
      const meldung = e instanceof Error ? e.message : String(e);
      setFehler(translateMeetingError(meldung, t));
      if (!nurNotizen) {
        await laden();
        // Nach einem gesperrten Start nur prüfen, niemals automatisch erneut starten.
        try {
          setModus(await api.startModus(plan.id, personen));
        } catch {
          /* Die ursprüngliche Fehlermeldung bleibt sichtbar. */
        }
      }
    } finally {
      beenden();
    }
  };

  const gruende = modus?.modus === "nur_notizen" ? startGruende(modus) : null;

  return (
    <section className="plg-karte" aria-labelledby={titelId}>
      <h2 id={titelId} className="plg-karte__titel">
        {t("planung.beginnen.titel")}
      </h2>
      {!plan.transcription_planned && (
        <p className="plg-meldung">{t("planung.beginnen.nichtGeplant")}</p>
      )}
      {ladeFehler ? (
        <div className="plg-meldung plg-meldung--fehler" role="alert">
          <p>{t("planung.einwilligungen.ladeFehler")}</p>
          <Button
            type="button"
            disabled={beschaeftigt}
            onClick={() => {
              void erneutLaden();
            }}
          >
            {t("planung.einwilligungen.erneutVersuchen")}
          </Button>
        </div>
      ) : (
        einwilligungen === null && (
          <p className="plg-meldung" role="status">
            {t("planung.einwilligungen.laden")}
          </p>
        )
      )}
      <ul className="plg-personen" data-testid="planung-start-anwesende">
        {anwesende.map((a) => {
          const stand = personStand(a, einwilligungen ?? []);
          const name =
            a.name ?? a.email ?? t("planung.beginnen.nichtIdentifiziert");
          return (
            <li
              key={a.id}
              className="plg-person"
              data-testid="planung-start-person"
            >
              <span className="plg-person__name">{name}</span>
              <span className="plg-person__stand">
                {t(
                  stand === "nicht_identifiziert"
                    ? "planung.beginnen.nichtIdentifiziert"
                    : `planung.einwilligungen.zustand.${stand}`,
                )}
              </span>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="sm"
                  disabled={beschaeftigt}
                  data-testid="planung-start-entfernen"
                  aria-label={t("planung.beginnen.entfernen", { name })}
                  onClick={() => {
                    if (gesperrt.current) return;
                    setAnwesende((liste) => liste.filter((p) => p.id !== a.id));
                    setModus(null);
                  }}
                >
                  {t("planung.beginnen.entfernen", { name })}
                </Button>
                {plan.transcription_planned &&
                  a.email !== null &&
                  (["zugestimmt", "abgelehnt"] as const).map(
                    (state) =>
                      stand !== state && (
                        <Button
                          key={state}
                          type="button"
                          size="sm"
                          disabled={beschaeftigt}
                          data-testid={
                            state === "zugestimmt"
                              ? "planung-start-zustimmen"
                              : "planung-start-ablehnen"
                          }
                          onClick={() => {
                            if (a.email !== null) {
                              void setzen(a.email, state);
                            }
                          }}
                        >
                          {t(
                            state === "zugestimmt"
                              ? "planung.beginnen.zustimmen"
                              : "planung.beginnen.ablehnen",
                          )}
                        </Button>
                      ),
                  )}
              </div>
              {a.email === null && (
                <p className="plg-karte__hilfe">
                  {t("planung.beginnen.nichtIdentifiziertHinweis")}
                </p>
              )}
            </li>
          );
        })}
      </ul>
      <div className="flex flex-wrap gap-2">
        <input
          type="text"
          value={hinzufuegenText}
          disabled={beschaeftigt}
          data-testid="planung-start-hinzu-eingabe"
          aria-label={t("planung.beginnen.hinzufuegenLabel")}
          aria-invalid={hinzufuegenFehler !== null}
          onChange={(e) => {
            setHinzufuegenText(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
              e.preventDefault();
              hinzufuegen();
            }
          }}
        />
        <Button
          type="button"
          disabled={beschaeftigt}
          data-testid="planung-start-hinzu"
          onClick={hinzufuegen}
        >
          {t("planung.beginnen.hinzufuegen")}
        </Button>
      </div>
      {hinzufuegenFehler && <p role="alert">{hinzufuegenFehler}</p>}
      {fehler && (
        <p
          className="plg-meldung plg-meldung--fehler"
          role="alert"
          data-testid="planung-start-fehler"
        >
          {fehler}
        </p>
      )}
      {gruende && (
        <div
          className="plg-meldung"
          role="status"
          data-testid="planung-start-gruende"
        >
          {gruende.nichtGeplant && (
            <p>{t("planung.beginnen.grund.nichtGeplant")}</p>
          )}
          {gruende.offen.length > 0 && (
            <p>
              {t("planung.beginnen.grund.offen", {
                personen: gruende.offen.join(", "),
              })}
            </p>
          )}
          {gruende.abgelehnt.length > 0 && (
            <p>
              {t("planung.beginnen.grund.abgelehnt", {
                personen: gruende.abgelehnt.join(", "),
              })}
            </p>
          )}
          {gruende.nichtIdentifiziert > 0 && (
            <p>
              {t("planung.beginnen.grund.unbekannt", {
                count: gruende.nichtIdentifiziert,
              })}
            </p>
          )}
          <p>{t("planung.beginnen.nurNotizenAngebot")}</p>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          data-testid="planung-start-los"
          disabled={
            !plan.transcription_planned || eventKey === null || beschaeftigt
          }
          onClick={() => {
            void starten(false);
          }}
        >
          {t("planung.beginnen.los")}
        </Button>
        <Button
          type="button"
          data-testid="planung-start-notizen"
          disabled={beschaeftigt}
          onClick={() => {
            void starten(true);
          }}
        >
          {t("planung.beginnen.nurNotizen")}
        </Button>
        {onAbbrechen && (
          <Button
            type="button"
            variant="ghost"
            disabled={beschaeftigt}
            data-testid="planung-start-abbrechen"
            onClick={onAbbrechen}
          >
            {t("planung.beginnen.abbrechen")}
          </Button>
        )}
      </div>
      {eventKey === null && (
        <p className="plg-meldung">{t("planung.beginnen.keinTermin")}</p>
      )}
    </section>
  );
};
