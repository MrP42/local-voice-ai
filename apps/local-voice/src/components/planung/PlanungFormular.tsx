import React from "react";
import { useTranslation } from "react-i18next";
import { Switch } from "@fluentui/react-components";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";
import type { GoalType } from "@/bindings";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import { Textarea } from "../ui/Textarea";
import {
  EMPFOHLENE_DAUERN,
  ZIEL_ARTEN,
  agendaSumme,
  klassifizierungen,
  minutenAusText,
  type PlanForm,
  type Weiterleiten,
} from "./logik";
import { Abschnitt, Feld } from "./Teile";

interface Props {
  form: PlanForm;
  onChange: (form: PlanForm) => void;
  /** Gesperrt: die Planung ist durchgeführt, das Protokoll bezieht sich auf diese Fassung. */
  gesperrt: boolean;
  /** Der Beginn kommt aus dem Termin in Outlook und wird dort geändert. */
  beginnAusTermin: boolean;
}

function verschiebe<T>(liste: T[], von: number, nach: number): T[] {
  if (nach < 0 || nach >= liste.length) return liste;
  const kopie = [...liste];
  const [x] = kopie.splice(von, 1);
  kopie.splice(nach, 0, x);
  return kopie;
}

/** Planungsformular nach Schritt 1 des Konzepts: Ziel, Ergebnis, Agenda, Rollen, Unterlagen, Hinweise. */
export const PlanungFormular: React.FC<Props> = ({
  form,
  onChange,
  gesperrt,
  beginnAusTermin,
}) => {
  const { t } = useTranslation();
  const set = (teil: Partial<PlanForm>) => onChange({ ...form, ...teil });
  const summe = agendaSumme(form);
  const dauer = minutenAusText(form.duration_min);

  const zielOptionen = ZIEL_ARTEN.map((a) => ({
    value: a,
    label: t(`planung.formular.zielArt.${a}`),
  }));
  const klassOptionen = klassifizierungen(form.classification).map((k) => ({
    value: k,
    label: k,
  }));
  const weiterOptionen: { value: Weiterleiten | "-"; label: string }[] = [
    { value: "-", label: t("planung.formular.weiterleiten.offen") },
    { value: "ja", label: t("planung.formular.weiterleiten.ja") },
    { value: "nein", label: t("planung.formular.weiterleiten.nein") },
  ];

  return (
    <fieldset
      className="plg-formular"
      disabled={gesperrt}
      aria-label={t("planung.formular.ariaLabel")}
      data-testid="planung-formular"
    >
      <Abschnitt titel={t("planung.formular.grundlagen")}>
        <Feld label={t("planung.formular.titel")}>
          {({ id }) => (
            <Input
              id={id}
              value={form.title}
              placeholder={t("planung.formular.titelPlatzhalter")}
              onChange={(e) => set({ title: e.target.value })}
              data-testid="planung-titel"
            />
          )}
        </Feld>
        <div className="plg-reihe">
          <Feld label={t("planung.formular.zielArtLabel")}>
            {() => (
              <Select
                value={form.goal_type}
                options={zielOptionen}
                ariaLabel={t("planung.formular.zielArtLabel")}
                onChange={(v) => v && set({ goal_type: v as GoalType })}
                disabled={gesperrt}
                isClearable={false}
              />
            )}
          </Feld>
          <Feld label={t("planung.formular.klassifizierung")}>
            {() => (
              <Select
                value={form.classification}
                options={klassOptionen}
                ariaLabel={t("planung.formular.klassifizierung")}
                onChange={(v) => v && set({ classification: v })}
                disabled={gesperrt}
                isClearable={false}
              />
            )}
          </Feld>
        </div>
        <Feld
          label={t("planung.formular.ziel")}
          hilfe={t("planung.formular.zielHilfe")}
        >
          {({ id, hilfeId }) => (
            <Textarea
              id={id}
              aria-describedby={hilfeId}
              rows={2}
              value={form.goal_text}
              onChange={(e) => set({ goal_text: e.target.value })}
              data-testid="planung-ziel"
            />
          )}
        </Feld>
        <Feld
          label={t("planung.formular.ergebnis")}
          hilfe={t("planung.formular.ergebnisHilfe")}
        >
          {({ id, hilfeId }) => (
            <Textarea
              id={id}
              aria-describedby={hilfeId}
              rows={2}
              value={form.expected_result}
              onChange={(e) => set({ expected_result: e.target.value })}
              data-testid="planung-ergebnis"
            />
          )}
        </Feld>
        <div className="plg-reihe">
          <Feld label={t("planung.formular.dauer")}>
            {({ id }) => (
              <div className="plg-dauer">
                <Input
                  id={id}
                  inputMode="numeric"
                  value={form.duration_min}
                  onChange={(e) => set({ duration_min: e.target.value })}
                  data-testid="planung-dauer"
                />
                {EMPFOHLENE_DAUERN.map((d) => (
                  <Button
                    key={d}
                    type="button"
                    size="sm"
                    variant={dauer === d ? "primary-soft" : "ghost"}
                    aria-pressed={dauer === d}
                    onClick={() => set({ duration_min: String(d) })}
                  >
                    {t("planung.formular.dauerKurz", { min: d })}
                  </Button>
                ))}
              </div>
            )}
          </Feld>
          <Feld
            label={t("planung.formular.beginn")}
            hilfe={
              beginnAusTermin
                ? t("planung.formular.beginnAusTermin")
                : t("planung.formular.beginnHilfe")
            }
          >
            {({ id, hilfeId }) => (
              <Input
                id={id}
                type="time"
                aria-describedby={hilfeId}
                value={form.beginn}
                readOnly={beginnAusTermin}
                onChange={(e) => set({ beginn: e.target.value })}
                data-testid="planung-beginn"
              />
            )}
          </Feld>
        </div>
      </Abschnitt>

      <Abschnitt
        titel={t("planung.formular.agenda")}
        hilfe={t("planung.formular.agendaHilfe")}
        testId="planung-agenda"
      >
        {form.agenda.length === 0 && (
          <p className="plg-leer">{t("planung.formular.agendaLeer")}</p>
        )}
        <ol className="plg-liste-felder">
          {form.agenda.map((z, i) => (
            <li
              key={i}
              className="plg-agendazeile"
              data-testid="planung-agendazeile"
            >
              <span className="plg-agendazeile__nr" aria-hidden="true">
                {i + 1}.
              </span>
              <Input
                aria-label={t("planung.formular.agendaThema", { nr: i + 1 })}
                placeholder={t("planung.formular.agendaThemaPlatzhalter")}
                value={z.topic}
                onChange={(e) =>
                  set({
                    agenda: form.agenda.map((x, k) =>
                      k === i ? { ...x, topic: e.target.value } : x,
                    ),
                  })
                }
                className="plg-agendazeile__thema"
              />
              <Input
                aria-label={t("planung.formular.agendaMinuten", { nr: i + 1 })}
                inputMode="numeric"
                value={z.minutes}
                onChange={(e) =>
                  set({
                    agenda: form.agenda.map((x, k) =>
                      k === i ? { ...x, minutes: e.target.value } : x,
                    ),
                  })
                }
                className="plg-agendazeile__min"
              />
              <Input
                aria-label={t("planung.formular.agendaWer", { nr: i + 1 })}
                placeholder={t("planung.formular.agendaWerPlatzhalter")}
                value={z.owner}
                onChange={(e) =>
                  set({
                    agenda: form.agenda.map((x, k) =>
                      k === i ? { ...x, owner: e.target.value } : x,
                    ),
                  })
                }
                className="plg-agendazeile__wer"
              />
              <span className="plg-agendazeile__tools">
                <IconButton
                  icon={<ArrowUp size={16} aria-hidden="true" />}
                  label={t("planung.formular.nachOben")}
                  disabled={i === 0}
                  onClick={() =>
                    set({ agenda: verschiebe(form.agenda, i, i - 1) })
                  }
                />
                <IconButton
                  icon={<ArrowDown size={16} aria-hidden="true" />}
                  label={t("planung.formular.nachUnten")}
                  disabled={i === form.agenda.length - 1}
                  onClick={() =>
                    set({ agenda: verschiebe(form.agenda, i, i + 1) })
                  }
                />
                <IconButton
                  icon={<Trash2 size={16} aria-hidden="true" />}
                  label={t("planung.formular.entfernen")}
                  onClick={() =>
                    set({ agenda: form.agenda.filter((_, k) => k !== i) })
                  }
                />
              </span>
            </li>
          ))}
        </ol>
        <div className="plg-zeile-aktion">
          <Button
            type="button"
            size="sm"
            icon={<Plus size={16} aria-hidden="true" />}
            onClick={() =>
              set({
                agenda: [
                  ...form.agenda,
                  { topic: "", minutes: "5", owner: "" },
                ],
              })
            }
            data-testid="planung-agenda-neu"
          >
            {t("planung.formular.agendaNeu")}
          </Button>
          {form.agenda.length > 0 && (
            <span
              className={`plg-summe ${summe > dauer ? "plg-summe--zuviel" : ""}`}
              role="status"
              data-testid="planung-summe"
            >
              {t("planung.formular.agendaSumme", { summe, dauer })}
            </span>
          )}
        </div>
      </Abschnitt>

      <Abschnitt titel={t("planung.formular.rollen")}>
        <div className="plg-reihe plg-reihe--drei">
          <Feld label={t("planung.formular.moderation")}>
            {({ id }) => (
              <Input
                id={id}
                value={form.moderation}
                onChange={(e) => set({ moderation: e.target.value })}
              />
            )}
          </Feld>
          <Feld label={t("planung.formular.ergebnissicherung")}>
            {({ id }) => (
              <Input
                id={id}
                value={form.ergebnissicherung}
                onChange={(e) => set({ ergebnissicherung: e.target.value })}
              />
            )}
          </Feld>
          <Feld label={t("planung.formular.entscheidung")}>
            {({ id }) => (
              <Input
                id={id}
                value={form.entscheidung}
                onChange={(e) => set({ entscheidung: e.target.value })}
              />
            )}
          </Feld>
        </div>
      </Abschnitt>

      <Abschnitt
        titel={t("planung.formular.unterlagen")}
        hilfe={t("planung.formular.unterlagenHilfe")}
      >
        <ul className="plg-liste-felder">
          {form.materials.map((m, i) => (
            <li key={i} className="plg-unterlage">
              <Input
                aria-label={t("planung.formular.unterlageName", { nr: i + 1 })}
                placeholder={t("planung.formular.unterlageNamePlatzhalter")}
                value={m.label}
                onChange={(e) =>
                  set({
                    materials: form.materials.map((x, k) =>
                      k === i ? { ...x, label: e.target.value } : x,
                    ),
                  })
                }
              />
              <Input
                aria-label={t("planung.formular.unterlageLink", { nr: i + 1 })}
                placeholder={t("planung.formular.unterlageLinkPlatzhalter")}
                value={m.url}
                onChange={(e) =>
                  set({
                    materials: form.materials.map((x, k) =>
                      k === i ? { ...x, url: e.target.value } : x,
                    ),
                  })
                }
              />
              <Input
                type="date"
                aria-label={t("planung.formular.unterlageFrist", { nr: i + 1 })}
                value={m.due_date}
                onChange={(e) =>
                  set({
                    materials: form.materials.map((x, k) =>
                      k === i ? { ...x, due_date: e.target.value } : x,
                    ),
                  })
                }
              />
              <IconButton
                icon={<Trash2 size={16} aria-hidden="true" />}
                label={t("planung.formular.entfernen")}
                onClick={() =>
                  set({ materials: form.materials.filter((_, k) => k !== i) })
                }
              />
            </li>
          ))}
        </ul>
        <div className="plg-zeile-aktion">
          <Button
            type="button"
            size="sm"
            icon={<Plus size={16} aria-hidden="true" />}
            onClick={() =>
              set({
                materials: [
                  ...form.materials,
                  { label: "", url: "", due_date: "" },
                ],
              })
            }
          >
            {t("planung.formular.unterlageNeu")}
          </Button>
        </div>
      </Abschnitt>

      <Abschnitt titel={t("planung.formular.hinweise")}>
        <div className="plg-reihe">
          <Feld label={t("planung.formular.weiterleitenLabel")}>
            {() => (
              <Select
                value={form.weiterleiten === "" ? "-" : form.weiterleiten}
                options={weiterOptionen}
                ariaLabel={t("planung.formular.weiterleitenLabel")}
                onChange={(v) =>
                  set({
                    weiterleiten: v === "ja" || v === "nein" ? v : "",
                  })
                }
                disabled={gesperrt}
                isClearable={false}
              />
            )}
          </Feld>
          <Feld label={t("planung.formular.ergebnisfrist")}>
            {({ id }) => (
              <Input
                id={id}
                type="date"
                value={form.ergebnisfrist}
                onChange={(e) => set({ ergebnisfrist: e.target.value })}
              />
            )}
          </Feld>
        </div>
        <Feld label={t("planung.formular.barrierefreiheit")}>
          {({ id }) => (
            <Input
              id={id}
              value={form.barrierefreiheit}
              onChange={(e) => set({ barrierefreiheit: e.target.value })}
            />
          )}
        </Feld>
        <Feld label={t("planung.formular.ablageort")}>
          {({ id }) => (
            <Input
              id={id}
              value={form.ablageort}
              onChange={(e) => set({ ablageort: e.target.value })}
            />
          )}
        </Feld>
      </Abschnitt>

      <Abschnitt titel={t("planung.formular.transkription")}>
        <Switch
          checked={form.transcription_planned}
          onChange={(_e, d) => set({ transcription_planned: d.checked })}
          label={t("planung.formular.transkriptionPlanen")}
          data-testid="planung-transkription"
        />
        <p className="plg-feld__hilfe">
          {t("planung.formular.transkriptionHilfe")}
        </p>
      </Abschnitt>
    </fieldset>
  );
};
