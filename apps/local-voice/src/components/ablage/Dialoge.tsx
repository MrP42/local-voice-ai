import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AblageOrdner, AblageEinstellungen, KiStufe } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Select, type SelectOption } from "../ui/Select";
import { KLASSEN, geerbteKlasse, klasseRang } from "./logik";

/** Eingabe eines Namens (neuer Ordner, Umbenennen). Enter speichert, Escape schließt. */
export const NameDialog: React.FC<{
  open: boolean;
  titel: string;
  beschriftung: string;
  start: string;
  aktion: string;
  busy: boolean;
  onOk(name: string): void;
  onClose(): void;
}> = ({ open, titel, beschriftung, start, aktion, busy, onOk, onClose }) => {
  const { t } = useTranslation();
  const [name, setName] = useState(start);
  const feld = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (open) setName(start);
  }, [open, start]);
  const gueltig = name.trim().length > 0;
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && onClose()}
      title={titel}
      closeLabel={t("ablage.schliessen")}
      initialFocusRef={feld}
      footer={
        <>
          <Button
            variant="primary"
            type="submit"
            form="ablage-name-form"
            disabled={busy || !gueltig}
            data-testid="ablage-name-ok"
          >
            {aktion}
          </Button>
          <Button type="button" onClick={onClose} disabled={busy}>
            {t("ablage.abbrechen")}
          </Button>
        </>
      }
    >
      <form
        id="ablage-name-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (gueltig && !busy) onOk(name.trim());
        }}
      >
        <label className="ablage-mut" htmlFor="ablage-name-feld">
          {beschriftung}
        </label>
        <Input
          id="ablage-name-feld"
          ref={feld}
          value={name}
          maxLength={120}
          autoComplete="off"
          onChange={(e) => setName(e.target.value)}
          data-testid="ablage-name-feld"
        />
      </form>
    </Dialog>
  );
};

/** Bestätigung mit klarer Benennung der Folge (endgültig löschen, Papierkorb leeren, Wechsel lokal). */
export const BestaetigungDialog: React.FC<{
  open: boolean;
  titel: string;
  text: React.ReactNode;
  aktion: string;
  gefahr?: boolean;
  busy: boolean;
  onOk(): void;
  onClose(): void;
}> = ({ open, titel, text, aktion, gefahr, busy, onOk, onClose }) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && onClose()}
      title={titel}
      closeLabel={t("ablage.schliessen")}
      footer={
        <>
          <Button
            variant={gefahr ? "danger" : "primary"}
            onClick={onOk}
            disabled={busy}
            data-testid="ablage-bestaetigen"
          >
            {aktion}
          </Button>
          <Button onClick={onClose} disabled={busy}>
            {t("ablage.abbrechen")}
          </Button>
        </>
      }
    >
      <div className="text-sm">{text}</div>
    </Dialog>
  );
};

/** Schalter nach der Vorschau: `button role=switch`, Fläche 52×44 px. */
export const Schalter: React.FC<{
  an: boolean;
  label: string;
  disabled?: boolean;
  onChange(an: boolean): void;
}> = ({ an, label, disabled, onChange }) => (
  <button
    type="button"
    role="switch"
    aria-checked={an}
    aria-label={label}
    disabled={disabled}
    className="ablage-tg"
    onClick={() => onChange(!an)}
  />
);

/** Optionsgruppe mit Pfeiltasten (An/Aus/Gesperrt). */
export const KiWahl: React.FC<{
  wert: KiStufe;
  disabled?: boolean;
  labelledBy: string;
  /** Bekommt die gewählte Option (erster Fokus beim Öffnen des Dialogs). */
  fokusRef?: React.RefObject<HTMLButtonElement>;
  onChange(w: KiStufe): void;
}> = ({ wert, disabled, labelledBy, fokusRef, onChange }) => {
  const { t } = useTranslation();
  const stufen: KiStufe[] = ["an", "aus", "gesperrt"];
  const knoepfe = useRef<(HTMLButtonElement | null)[]>([]);
  const waehle = (i: number) => {
    const n = (i + stufen.length) % stufen.length;
    onChange(stufen[n]);
    knoepfe.current[n]?.focus();
  };
  return (
    <div
      className="ablage-seg"
      role="radiogroup"
      aria-labelledby={labelledBy}
      onKeyDown={(e) => {
        const i = stufen.indexOf(wert);
        if (e.key === "ArrowRight" || e.key === "ArrowDown") {
          e.preventDefault();
          waehle(i + 1);
        } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
          e.preventDefault();
          waehle(i - 1);
        } else if (e.key === "Home") {
          e.preventDefault();
          waehle(0);
        } else if (e.key === "End") {
          e.preventDefault();
          waehle(stufen.length - 1);
        }
      }}
    >
      {stufen.map((s, i) => (
        <button
          key={s}
          ref={(el) => {
            knoepfe.current[i] = el;
            if (fokusRef && wert === s) {
              (
                fokusRef as React.MutableRefObject<HTMLButtonElement | null>
              ).current = el;
            }
          }}
          type="button"
          role="radio"
          aria-checked={wert === s}
          tabIndex={wert === s ? 0 : -1}
          disabled={disabled}
          className={s === "gesperrt" ? "ablage-seg__gesperrt" : undefined}
          onClick={() => onChange(s)}
          data-testid={`ablage-ki-${s}`}
        >
          {t(`ablage.ki.stufe.${s}`)}
        </button>
      ))}
    </div>
  );
};

export interface EinstellungenAenderung {
  ki?: KiStufe | null;
  einstellungen: AblageEinstellungen;
}

/**
 * Ordnereinstellungen: KI-Freigabe (gerätelokal), Datenklasse, Nur dieses Gerät, KI-Entwürfe.
 * Ein Wechsel von „Nur dieses Gerät“ braucht eine Bestätigung mit Folge im selben Dialog.
 */
export const OrdnerEinstellungen: React.FC<{
  open: boolean;
  ordner: AblageOrdner | null;
  alle: AblageOrdner[];
  busy: boolean;
  onSpeichern(a: EinstellungenAenderung): void;
  onClose(): void;
}> = ({ open, ordner, alle, busy, onSpeichern, onClose }) => {
  const { t } = useTranslation();
  const [ki, setKi] = useState<KiStufe>("aus");
  const [eigeneKi, setEigeneKi] = useState(false);
  const [klasse, setKlasse] = useState("erbt");
  const [nur, setNur] = useState(false);
  const [entwuerfe, setEntwuerfe] = useState(false);
  const [bestaetigen, setBestaetigen] = useState(false);
  const ordnerId = ordner?.id ?? null;
  useEffect(() => {
    // Nur beim Öffnen oder Ordnerwechsel zurücksetzen: ein Neuladen im Hintergrund
    // (neues Objekt, gleiche Kennung) darf ungespeicherte Eingaben nicht verwerfen.
    if (!open || !ordner) return;
    const z = ordner.ki.zustand;
    setKi(z === "an" || z === "gesperrt" ? z : "aus");
    setEigeneKi(!ordner.ki.von && !ordner.ki.vorgabe);
    setKlasse(ordner.datenklasse);
    setNur(ordner.nur_geraet);
    setEntwuerfe(ordner.zielordner_ki);
    setBestaetigen(false);
  }, [open, ordnerId]);
  const kiRef = useRef<HTMLButtonElement>(null);
  const unklar = ordner?.ki.zustand === "unklar";
  if (!ordner) return null;
  const hatEltern = !!ordner.eltern;
  const mindest = geerbteKlasse(alle, ordner.id);
  const klassenOptionen: SelectOption[] = [
    {
      value: "erbt",
      label: hatEltern ? t("ablage.klasse.erbt") : t("ablage.klasse.standard"),
    },
    ...KLASSEN.map((k) => ({
      value: k,
      label: t(`ablage.klasse.lang.${k}`),
      isDisabled: hatEltern && klasseRang(k) < klasseRang(mindest),
    })),
  ];
  const nurGeaendert = nur !== ordner.nur_geraet;
  const kiGeaendert =
    !unklar &&
    (ki !== ordner.ki.zustand ||
      eigeneKi !== (!ordner.ki.von && !ordner.ki.vorgabe));
  const speichern = () => {
    const einstellungen: AblageEinstellungen = {
      datenklasse: klasse !== ordner.datenklasse ? klasse : null,
      nur_geraet: nurGeaendert ? nur : null,
      zielordner_ki: entwuerfe !== ordner.zielordner_ki ? entwuerfe : null,
    };
    onSpeichern({
      ki: kiGeaendert ? (eigeneKi ? ki : null) : undefined,
      einstellungen,
    });
  };
  const nurGeerbt = ordner.nur_geraet_wirksam && !ordner.nur_geraet;
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && onClose()}
      title={t("ablage.einstellungen.titel", { name: ordner.name })}
      closeLabel={t("ablage.schliessen")}
      initialFocusRef={unklar || bestaetigen ? undefined : kiRef}
      footer={
        bestaetigen ? (
          <>
            <Button
              variant="primary"
              onClick={speichern}
              disabled={busy}
              data-testid="ablage-einstellungen-bestaetigen"
            >
              {t(
                nur
                  ? "ablage.einstellungen.nurBestaetigen"
                  : "ablage.einstellungen.syncBestaetigen",
              )}
            </Button>
            <Button onClick={() => setBestaetigen(false)} disabled={busy}>
              {t("ablage.zurueck")}
            </Button>
          </>
        ) : (
          <>
            <Button
              variant="primary"
              disabled={busy}
              onClick={() =>
                nurGeaendert ? setBestaetigen(true) : speichern()
              }
              data-testid="ablage-einstellungen-speichern"
            >
              {t("ablage.speichern")}
            </Button>
            <Button onClick={onClose} disabled={busy}>
              {t("ablage.abbrechen")}
            </Button>
          </>
        )
      }
    >
      {bestaetigen ? (
        <div className="text-sm" role="alert">
          <p>
            {t(
              nur
                ? "ablage.einstellungen.nurFrage"
                : "ablage.einstellungen.syncFrage",
              { name: ordner.name },
            )}
          </p>
          <p className="ablage-mut" style={{ marginTop: 8 }}>
            {t(
              nur
                ? "ablage.einstellungen.nurFolge"
                : "ablage.einstellungen.syncFolge",
            )}
          </p>
        </div>
      ) : (
        <div>
          <div
            className="ablage-grp"
            style={{ padding: "0 0 4px" }}
            id="ablage-ki-label"
          >
            {t("ablage.ki.titel")}
          </div>
          {unklar && (
            <p className="ablage-mut" role="status">
              {t("ablage.ki.unklarHinweis")}
            </p>
          )}
          <KiWahl
            wert={ki}
            disabled={unklar}
            labelledBy="ablage-ki-label"
            fokusRef={kiRef}
            onChange={(w) => {
              setKi(w);
              setEigeneKi(true);
            }}
          />
          <p className="ablage-mut" style={{ margin: "6px 0 8px" }}>
            {t("ablage.ki.erklaerung")}
          </p>
          {hatEltern && eigeneKi && !unklar && (
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setEigeneKi(false);
                setKi("aus");
              }}
              data-testid="ablage-ki-erben"
            >
              {t("ablage.ki.erben")}
            </Button>
          )}
          <div
            className="ablage-grp"
            style={{ padding: "8px 0 4px" }}
            id="ablage-klasse-label"
          >
            {t("ablage.klasse.titel")}
          </div>
          <Select
            value={klasse}
            options={klassenOptionen}
            ariaLabel={t("ablage.klasse.titel")}
            onChange={(v) => v && setKlasse(v)}
          />
          <div className="ablage-sw">
            <span>
              {t("ablage.nurGeraet.titel")}
              <br />
              <span className="ablage-mut">
                {nurGeerbt
                  ? t("ablage.nurGeraet.geerbt")
                  : t("ablage.nurGeraet.hinweis")}
              </span>
            </span>
            <Schalter
              an={nur || nurGeerbt}
              disabled={nurGeerbt}
              label={t("ablage.nurGeraet.titel")}
              onChange={setNur}
            />
          </div>
          <div className="ablage-sw">
            <span>
              {t("ablage.entwuerfe.titel")}
              <br />
              <span className="ablage-mut">
                {t("ablage.entwuerfe.hinweis")}
              </span>
            </span>
            <Schalter
              an={entwuerfe}
              label={t("ablage.entwuerfe.titel")}
              onChange={setEntwuerfe}
            />
          </div>
        </div>
      )}
    </Dialog>
  );
};
