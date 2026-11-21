import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Consent, Plan, StartModus } from "@/bindings";
import { Button } from "../ui/Button";
import { AgendaZeitboxen } from "./AgendaZeitboxen";
import type { StartErgebnis } from "./StartPanel";
import { api } from "./api";
import {
  UNBEKANNT,
  anwesendeFuerBackend,
  anwesenderHinzufuegen,
  personStand,
  startGruende,
  type Anwesender,
} from "./durchfuehren";

interface Props {
  plan: Plan;
  lauf: StartErgebnis;
  /** Aktueller Stand der Einwilligungen; null bedeutet Laden oder Fehler. */
  einwilligungen: Consent[] | null;
  /** Die Seite lädt nach einer Änderung die Einwilligungen neu. */
  onEinwilligungGeaendert: () => void;
}

export const LaufPanel: React.FC<Props> = ({
  plan,
  lauf,
  einwilligungen,
  onEinwilligungGeaendert,
}) => {
  const { t } = useTranslation();
  const [anwesende, setAnwesende] = useState<Anwesender[]>(() =>
    lauf.anwesende.map((person, index) => ({
      id: `a${index + 1}`,
      email: person === UNBEKANNT ? null : person,
      name: null,
    })),
  );
  const [modus, setModus] = useState<StartModus | null>(null);
  const [modusFehler, setModusFehler] = useState(false);
  const [beschaeftigt, setBeschaeftigt] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  const [ergaenzenFehler, setErgaenzenFehler] = useState(false);
  const [hinzufuegenText, setHinzufuegenText] = useState("");
  const [hinzufuegenFehler, setHinzufuegenFehler] = useState<string | null>(
    null,
  );
  const gesperrt = useRef(false);

  useEffect(() => {
    let aktuell = true;
    setModus(null);
    setModusFehler(false);
    void api.startModus(plan.id, anwesendeFuerBackend(anwesende)).then(
      (ergebnis) => {
        if (aktuell) setModus(ergebnis);
      },
      () => {
        if (aktuell) setModusFehler(true);
      },
    );
    return () => {
      // Antworten einer überholten Prüfung dürfen den Stand nicht ersetzen.
      aktuell = false;
    };
  }, [plan.id, anwesende, einwilligungen]);

  const zustimmen = async (person: string) => {
    if (gesperrt.current) return;
    gesperrt.current = true;
    setBeschaeftigt(true);
    setFehler(null);
    try {
      await api.einwilligungSetzen({
        plan_id: plan.id,
        person,
        state: "zugestimmt",
        channel: "muendlich_bei_start",
        recorded_at: Date.now(),
      });
      onEinwilligungGeaendert();
    } catch {
      setFehler(t("planung.einwilligungen.schreibFehler"));
    } finally {
      gesperrt.current = false;
      setBeschaeftigt(false);
    }
  };

  const hinzufuegen = async () => {
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
    gesperrt.current = true;
    setBeschaeftigt(true);
    setErgaenzenFehler(false);
    try {
      await api.anwesendeErgaenzen(lauf.meetingId, plan.id, [
        ergebnis.liste[ergebnis.liste.length - 1].email!,
      ]);
      setAnwesende(ergebnis.liste);
      setHinzufuegenText("");
      setHinzufuegenFehler(null);
      setModus(null);
    } catch {
      setErgaenzenFehler(true);
    } finally {
      gesperrt.current = false;
      setBeschaeftigt(false);
    }
  };

  const pruefung = modus === null && !modusFehler;
  const aktiv =
    lauf.art === "transkription" && modus?.modus === "mit_transkription";
  const transkriptionGesperrt =
    lauf.art === "transkription" && modus?.modus === "nur_notizen";
  const gruende = transkriptionGesperrt && modus ? startGruende(modus) : null;

  return (
    <section data-testid="planung-lauf">
      <p
        role="status"
        data-testid="planung-lauf-modus"
        data-modus={pruefung ? "pruefung" : aktiv ? "transkription" : "notizen"}
      >
        {t(
          pruefung
            ? "planung.lauf.pruefung"
            : aktiv
              ? "planung.lauf.mitTranskription"
              : "planung.lauf.nurNotizen",
        )}
      </p>
      {modusFehler && (
        <p className="plg-meldung plg-meldung--fehler" role="alert">
          {t("planung.lauf.nichtPruefbar")}
        </p>
      )}
      {gruende && (
        <div
          className="plg-meldung plg-meldung--fehler"
          role="alert"
          data-testid="planung-lauf-gesperrt"
        >
          <p>{t("planung.lauf.gesperrt")}</p>
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
        </div>
      )}
      <ul className="plg-personen" data-testid="planung-lauf-anwesende">
        {anwesende.map((a) => {
          const stand = personStand(a, einwilligungen ?? []);
          const name =
            a.name ?? a.email ?? t("planung.beginnen.nichtIdentifiziert");
          return (
            <li
              key={a.id}
              className="plg-person"
              data-testid="planung-lauf-person"
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
                  data-testid="planung-lauf-entfernen"
                  aria-label={t("planung.beginnen.entfernen", { name })}
                  onClick={() => {
                    if (gesperrt.current) return;
                    setAnwesende((liste) => liste.filter((p) => p.id !== a.id));
                    setModus(null);
                  }}
                >
                  {t("planung.beginnen.entfernen", { name })}
                </Button>
                {transkriptionGesperrt &&
                  a.email !== null &&
                  (stand === "offen" || stand === "abgelehnt") && (
                    <Button
                      type="button"
                      size="sm"
                      disabled={beschaeftigt}
                      data-testid="planung-lauf-zustimmen"
                      aria-label={`${t("planung.beginnen.zustimmen")}: ${name}`}
                      onClick={() => {
                        if (a.email !== null) void zustimmen(a.email);
                      }}
                    >
                      {t("planung.beginnen.zustimmen")}
                    </Button>
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
      <h3 className="plg-karte__titel">{t("planung.lauf.spaeterTitel")}</h3>
      <div className="flex flex-wrap gap-2">
        <input
          type="text"
          value={hinzufuegenText}
          disabled={beschaeftigt}
          data-testid="planung-lauf-hinzu-eingabe"
          aria-label={t("planung.beginnen.hinzufuegenLabel")}
          aria-invalid={hinzufuegenFehler !== null}
          onChange={(e) => {
            setHinzufuegenText(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void hinzufuegen();
            }
          }}
        />
        <Button
          type="button"
          disabled={beschaeftigt}
          data-testid="planung-lauf-hinzu"
          onClick={() => {
            void hinzufuegen();
          }}
        >
          {t("planung.beginnen.hinzufuegen")}
        </Button>
      </div>
      {hinzufuegenFehler && <p role="alert">{hinzufuegenFehler}</p>}
      {ergaenzenFehler && (
        <p role="alert" data-testid="planung-lauf-ergaenzen-fehler">
          {t("planung.lauf.ergaenzenFehler")}
        </p>
      )}
      {fehler && (
        <p className="plg-meldung plg-meldung--fehler" role="alert">
          {fehler}
        </p>
      )}
      <AgendaZeitboxen agenda={plan.agenda} />
    </section>
  );
};
