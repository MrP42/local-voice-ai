import React from "react";
import { useTranslation } from "react-i18next";
import type { Attendee, PlanZumTermin } from "@/bindings";
import {
  anwesendeAusTermin,
  personStand,
} from "../components/planung/durchfuehren";

export const PlanHinweis: React.FC<{
  plaene: PlanZumTermin[];
  teilnehmende: readonly Attendee[];
}> = ({ plaene, teilnehmende }) => {
  const { t } = useTranslation();
  const anwesende = anwesendeAusTermin(teilnehmende);
  const standText = (state: ReturnType<typeof personStand>) =>
    t(
      state === "nicht_identifiziert"
        ? "meetings.prompt.plan.nichtIdentifiziert"
        : `planung.einwilligungen.zustand.${state}`,
    );

  return (
    <>
      {plaene.map(({ plan, einwilligungen }) => {
        const personen =
          anwesende.length > 0
            ? anwesende.map((a) => ({
                id: a.id,
                name: a.name ?? a.email,
                stand: personStand(a, einwilligungen),
              }))
            : einwilligungen.map((c, index) => ({
                id: `e${index}`,
                name: c.person,
                stand: c.state,
              }));
        const stand = { zugestimmt: 0, offen: 0, abgelehnt: 0, unbekannt: 0 };
        for (const person of personen) {
          if (person.stand === "nicht_identifiziert") stand.unbekannt += 1;
          else stand[person.stand] += 1;
        }
        const rollen = (
          ["moderation", "ergebnissicherung", "entscheidung"] as const
        ).filter((rolle) => plan.roles[rolle]);

        return (
          <section
            key={plan.id}
            data-testid="prompt-plan"
            className="space-y-2 border-t border-mid-gray/30 pt-2"
          >
            <div>
              <h2 className="text-xs font-semibold text-text/70">
                {t("meetings.prompt.plan.titel")}
              </h2>
              <p
                data-testid="prompt-plan-titel"
                className="text-sm font-semibold break-words"
              >
                {plan.title}
              </p>
            </div>
            <p data-testid="prompt-plan-ziel" className="text-sm break-words">
              {t(`planung.formular.zielArt.${plan.goal_type}`)}:{" "}
              {plan.goal_text}
            </p>
            {plan.expected_result && (
              <p className="text-xs break-words">
                {t("meetings.prompt.plan.ergebnis")}: {plan.expected_result}
              </p>
            )}
            {plan.agenda.length > 0 ? (
              <ol
                data-testid="prompt-plan-agenda"
                className="list-decimal pl-4 text-xs space-y-1"
              >
                {plan.agenda.map((punkt, index) => (
                  <li
                    key={index}
                    data-testid="prompt-plan-punkt"
                    className="break-words"
                  >
                    {t("planung.formular.dauerKurz", { min: punkt.minutes })}{" "}
                    {punkt.topic}
                    {punkt.owner && ` · ${punkt.owner}`}
                  </li>
                ))}
              </ol>
            ) : (
              <p className="text-xs text-text/70">
                {t("meetings.prompt.plan.agendaLeer")}
              </p>
            )}
            {rollen.length > 0 && (
              <dl
                data-testid="prompt-plan-rollen"
                className="text-xs space-y-1"
              >
                {rollen.map((rolle) => (
                  <div key={rolle} className="break-words">
                    <dt className="inline font-semibold">
                      {t(`planung.formular.${rolle}`)}:
                    </dt>{" "}
                    <dd className="inline">{plan.roles[rolle]}</dd>
                  </div>
                ))}
              </dl>
            )}
            <div data-testid="prompt-plan-einwilligungen" className="space-y-1">
              <h3 className="text-xs font-semibold">
                {t("meetings.prompt.plan.einwilligungen")}
              </h3>
              {!plan.transcription_planned ? (
                <p className="text-xs text-text/70">
                  {t("meetings.prompt.plan.keineTranskription")}
                </p>
              ) : (
                <>
                  <p data-testid="prompt-plan-stand" className="text-xs">
                    {t("meetings.prompt.plan.stand", stand)}
                  </p>
                  {personen.length > 0 ? (
                    <ul className="text-xs space-y-1">
                      {personen.map((person) => (
                        <li
                          key={person.id}
                          data-testid="prompt-plan-person"
                          data-stand={person.stand}
                          className="break-words"
                        >
                          {person.name ??
                            t("meetings.prompt.plan.nichtIdentifiziert")}
                          {" · "}
                          {standText(person.stand)}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="text-xs text-text/70">
                      {t("meetings.prompt.plan.keinePersonen")}
                    </p>
                  )}
                </>
              )}
            </div>
          </section>
        );
      })}
    </>
  );
};
