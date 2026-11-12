import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Eye, Settings } from "lucide-react";
import type {
  AblageAnbieterListe,
  AblageDatei,
  AblageEinstellungen,
  AblageOrdner,
  KiStufe,
} from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Select, type SelectOption } from "../ui/Select";
import {
  KLASSEN,
  anbieterNamen,
  cloudAenderung,
  geerbteKlasse,
  klasseRang,
  vertraulicheImUnterbaum,
  wirksameKlasseNeu,
} from "./logik";

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
      className="ablage-dlg"
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
      className="ablage-dlg"
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
  /** D7-Rest: Cloud-Ausnahmen, die erteilt oder entzogen werden (nur bei „Vertraulich“). */
  cloud?: { erteilen: string[]; entziehen: string[] };
}

type Schritt = "form" | "nur" | "cloud" | "verlassen";

/** Hinweisblock im Dialog (Info oder Warnung). */
const Hinweis: React.FC<{
  warn?: boolean;
  testid?: string;
  children: React.ReactNode;
}> = ({ warn, testid, children }) => (
  <div
    className={`ablage-fold${warn ? " ablage-fold--warn" : ""}`}
    data-testid={testid}
  >
    {children}
  </div>
);

/**
 * Ordnereinstellungen: KI-Freigabe (gerätelokal), Datenklasse, Cloud-Ausnahme, Nur dieses Gerät,
 * KI-Entwürfe. Wechsel von „Nur dieses Gerät“ und das Erteilen einer Cloud-Ausnahme brauchen eine
 * Bestätigung mit Folge im selben Dialog (die Frage ist dort der Titel). D7: Bei einem verknüpften
 * Ordner nur die Datenklasse (ohne „Erben“): die KI erreicht ihn nie, „Nur dieses Gerät“ gilt dort
 * immer, KI-Entwürfe landen nie dort.
 */
export const OrdnerEinstellungen: React.FC<{
  open: boolean;
  ordner: AblageOrdner | null;
  alle: AblageOrdner[];
  /** Für die Zahl in der Bestätigung der Cloud-Ausnahme. */
  dateien: AblageDatei[];
  /** Konfigurierte Anbieter für „Vertraulich“; `null` = ließ sich nicht laden. */
  anbieter: AblageAnbieterListe | null;
  busy: boolean;
  onSpeichern(a: EinstellungenAenderung): void;
  /** Wechsel zu „KI-Modelle & Anbieter“ (Regelwerk und Anbieter). */
  onRegelwerk(): void;
  onClose(): void;
}> = ({
  open,
  ordner,
  alle,
  dateien,
  anbieter,
  busy,
  onSpeichern,
  onRegelwerk,
  onClose,
}) => {
  const { t } = useTranslation();
  const [ki, setKi] = useState<KiStufe>("aus");
  const [eigeneKi, setEigeneKi] = useState(false);
  const [klasse, setKlasse] = useState("erbt");
  const [nur, setNur] = useState(false);
  const [entwuerfe, setEntwuerfe] = useState(false);
  const [schritt, setSchritt] = useState<Schritt>("form");
  const [cloudAn, setCloudAn] = useState(false);
  const [auswahl, setAuswahl] = useState<string[]>([]);
  const [cloudFehler, setCloudFehler] = useState(false);
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
    setSchritt("form");
    const eigene = ordner.cloud?.anbieter ?? [];
    setCloudAn(eigene.length > 0);
    setAuswahl(eigene);
    setCloudFehler(false);
  }, [open, ordnerId]);
  const kiRef = useRef<HTMLButtonElement>(null);
  const zurueckRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    // Bestätigungen: der Fokus liegt auf der sicheren Wahl („Zurück“).
    if (!open || schritt === "form") return;
    const frame = requestAnimationFrame(() => zurueckRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [open, schritt]);
  const unklar = ordner?.ki.zustand === "unklar";
  if (!ordner) return null;
  const hatEltern = !!ordner.eltern;
  const verknuepft = ordner.verknuepft;
  const mindest = geerbteKlasse(alle, ordner.id);
  const klassenOptionen: SelectOption[] = [
    ...(verknuepft
      ? []
      : [
          {
            value: "erbt",
            label: hatEltern
              ? t("ablage.klasse.erbt")
              : t("ablage.klasse.standard"),
          },
        ]),
    ...KLASSEN.map((k) => ({
      value: k,
      label: t(`ablage.klasse.lang.${k}`),
      isDisabled: hatEltern && klasseRang(k) < klasseRang(mindest),
    })),
  ];
  const nurGeaendert = nur !== ordner.nur_geraet;
  // Die KI erreicht verknüpfte Ordner nie: dort gibt es keine KI-Einstellung (nur einen Hinweis).
  const kiGeaendert =
    !verknuepft &&
    !unklar &&
    (ki !== ordner.ki.zustand ||
      eigeneKi !== (!ordner.ki.von && !ordner.ki.vorgabe));

  // --- Cloud-Ausnahme (D7-Rest) ---
  const bestand = ordner.cloud?.anbieter ?? [];
  const geerbt = ordner.cloud?.geerbt ?? [];
  const wirksamNeu = wirksameKlasseNeu(alle, ordner.id, klasse);
  const gespeichertVertraulich = ordner.klasse === "confidential";
  /** Schalter und Liste nur, solange die Klasse „Vertraulich“ gespeichert UND gewählt ist. */
  const cloudAktiv =
    !verknuepft && wirksamNeu === "confidential" && gespeichertVertraulich;
  const liste = anbieter?.anbieter ?? [];
  const namenVon = (ids: string[]) => anbieterNamen(ids, liste);
  const regelwerk = anbieter?.regelwerk ?? "eu";
  const regelwerkName = t(`ablage.cloud.regelwerk.${regelwerk}`, {
    defaultValue: regelwerk,
  });
  const waehlbare = liste.filter((a) => a.waehlbar);
  const regelSperre: "nur_lokal" | "keines" | "eu" | null = !anbieter
    ? null
    : regelwerk === "nur_lokal"
      ? "nur_lokal"
      : regelwerk === "keines"
        ? "keines"
        : waehlbare.length === 0
          ? "eu"
          : null;
  const kiGesperrt = ki === "gesperrt";
  // Erteilen ist gesperrt, solange kein Anbieter zulässig ist oder die KI „Gesperrt“ ist.
  // Eine bestehende Ausnahme bleibt sichtbar und lässt sich entziehen.
  const erteilenGesperrt = !anbieter || regelSperre !== null || kiGesperrt;
  const auswahlWirksam = cloudAktiv && cloudAn ? auswahl : [];
  const cloudAend = cloudAenderung(cloudAktiv ? bestand : [], auswahlWirksam);
  const cloudDa =
    cloudAend.erteilen.length > 0 || cloudAend.entziehen.length > 0;
  const zeilen = [
    ...liste,
    ...bestand
      .filter((id) => !liste.some((a) => a.id === id))
      .map((id) => ({ id, name: id, waehlbar: false, grund: "unbekannt" })),
  ];
  const zaehlung = vertraulicheImUnterbaum(alle, dateien, ordner.id);
  const aendern = (id: string, an: boolean) => {
    setCloudFehler(false);
    setAuswahl((alt) =>
      an ? [...new Set([...alt, id])] : alt.filter((x) => x !== id),
    );
  };

  const geaendert =
    kiGeaendert ||
    klasse !== ordner.datenklasse ||
    nurGeaendert ||
    entwuerfe !== ordner.zielordner_ki ||
    cloudDa;
  const speichern = () => {
    const einstellungen: AblageEinstellungen = {
      datenklasse: klasse !== ordner.datenklasse ? klasse : null,
      nur_geraet: nurGeaendert ? nur : null,
      zielordner_ki: entwuerfe !== ordner.zielordner_ki ? entwuerfe : null,
    };
    onSpeichern({
      ki: kiGeaendert ? (eigeneKi ? ki : null) : undefined,
      einstellungen,
      cloud: cloudDa ? cloudAend : undefined,
    });
  };
  /** Nächster Schritt nach „Speichern“ oder nach einer Bestätigung. */
  const weiter = (von: Schritt) => {
    if (von === "form") {
      if (cloudAktiv && cloudAn && auswahl.length === 0) {
        setCloudFehler(true);
        return;
      }
      if (nurGeaendert) return setSchritt("nur");
    }
    if (von !== "cloud" && cloudAend.erteilen.length > 0)
      return setSchritt("cloud");
    speichern();
  };
  const wechselZuRegelwerk = () =>
    geaendert ? setSchritt("verlassen") : onRegelwerk();
  const nurGeerbt = ordner.nur_geraet_wirksam && !ordner.nur_geraet;

  const titel =
    schritt === "nur"
      ? t(
          nur
            ? "ablage.einstellungen.nurFrage"
            : "ablage.einstellungen.syncFrage",
          {
            name: ordner.name,
          },
        )
      : schritt === "cloud"
        ? t("ablage.cloud.frage", { name: ordner.name })
        : schritt === "verlassen"
          ? t("ablage.cloud.verlassenTitel")
          : t("ablage.einstellungen.titel", { name: ordner.name });
  const beschreibung =
    schritt === "nur"
      ? t(
          nur
            ? "ablage.einstellungen.nurFolge"
            : "ablage.einstellungen.syncFolge",
        )
      : schritt === "cloud"
        ? t(
            zaehlung.dateien === 0
              ? "ablage.cloud.umfangNull"
              : "ablage.cloud.umfang",
            { count: zaehlung.dateien, name: ordner.name },
          )
        : schritt === "verlassen"
          ? t("ablage.cloud.verlassenText")
          : undefined;
  const zurueck = (
    <Button
      ref={zurueckRef}
      onClick={() => setSchritt("form")}
      disabled={busy}
      data-testid="ablage-einstellungen-zurueck"
    >
      {t("ablage.zurueck")}
    </Button>
  );
  const fuss =
    schritt === "nur" ? (
      <>
        <Button
          variant="primary"
          onClick={() => weiter("nur")}
          disabled={busy}
          data-testid="ablage-einstellungen-bestaetigen"
        >
          {cloudAend.erteilen.length > 0
            ? t("ablage.einstellungen.weiter")
            : t(
                nur
                  ? "ablage.einstellungen.nurBestaetigen"
                  : "ablage.einstellungen.syncBestaetigen",
              )}
        </Button>
        {zurueck}
      </>
    ) : schritt === "cloud" ? (
      <>
        <Button
          variant="primary"
          onClick={() => weiter("cloud")}
          disabled={busy}
          data-testid="ablage-cloud-erteilen"
        >
          {t("ablage.cloud.erteilen")}
        </Button>
        {zurueck}
      </>
    ) : schritt === "verlassen" ? (
      <>
        <Button
          variant="primary"
          onClick={onRegelwerk}
          disabled={busy}
          data-testid="ablage-cloud-verlassen"
        >
          {t("ablage.cloud.verlassenAktion")}
        </Button>
        {zurueck}
      </>
    ) : (
      <>
        <Button
          variant="primary"
          disabled={busy}
          onClick={() => weiter("form")}
          data-testid="ablage-einstellungen-speichern"
        >
          {t("ablage.speichern")}
        </Button>
        <Button onClick={onClose} disabled={busy}>
          {t("ablage.abbrechen")}
        </Button>
      </>
    );

  const cloudAbschnitt = (): React.ReactNode => {
    const eigeneNamen = namenVon(bestand).join(", ");
    if (verknuepft) return null;
    if (wirksamNeu === "confidential" && !gespeichertVertraulich)
      return (
        <Hinweis testid="ablage-cloud-erst-speichern">
          {t("ablage.cloud.erstSpeichern")}
        </Hinweis>
      );
    if (!cloudAktiv)
      return (
        <>
          <Hinweis testid="ablage-cloud-braucht-keine">
            {wirksamNeu === "personal"
              ? t("ablage.cloud.personal")
              : t("ablage.cloud.braucht", {
                  klasse: t(`ablage.klasse.lang.${wirksamNeu}`, {
                    defaultValue: wirksamNeu,
                  }),
                })}
          </Hinweis>
          {bestand.length > 0 && (
            <Hinweis warn testid="ablage-cloud-verfaellt">
              <b>{t("ablage.cloud.verfaellt", { anbieter: eigeneNamen })}</b>
            </Hinweis>
          )}
        </>
      );
    const sperrText = regelSperre
      ? t(`ablage.cloud.sperre.${regelSperre}`)
      : kiGesperrt
        ? t("ablage.cloud.sperre.ki")
        : null;
    return (
      <>
        <div className="ablage-sw">
          <span>
            {t("ablage.cloud.schalter")}
            <br />
            <span className="ablage-mut">
              {t("ablage.cloud.schalterHinweis")}
            </span>
          </span>
          <Schalter
            an={cloudAn}
            disabled={!cloudAn && erteilenGesperrt}
            label={t("ablage.cloud.schalter")}
            onChange={(a) => {
              setCloudAn(a);
              setCloudFehler(false);
            }}
          />
        </div>
        {!anbieter && (
          <Hinweis warn testid="ablage-cloud-nicht-geladen">
            {t("ablage.cloud.nichtGeladen")}
          </Hinweis>
        )}
        {sperrText && (
          <Hinweis testid="ablage-cloud-sperre">{sperrText}</Hinweis>
        )}
        {geerbt.length > 0 && (
          <Hinweis testid="ablage-cloud-geerbt">
            {t("ablage.cloud.geerbt", {
              anbieter: namenVon(geerbt).join(", "),
            })}
          </Hinweis>
        )}
        {cloudAn && (
          <>
            <Hinweis testid="ablage-cloud-info">
              {t("ablage.cloud.info")}
            </Hinweis>
            <fieldset
              className="ablage-cbs"
              aria-describedby={cloudFehler ? "ablage-cloud-fehler" : undefined}
              data-testid="ablage-cloud-anbieter"
            >
              <legend className="ablage-grp" style={{ padding: "8px 0 4px" }}>
                {t("ablage.cloud.anbieter", { regelwerk: regelwerkName })}
              </legend>
              {zeilen.map((z, i) => {
                const an = auswahl.includes(z.id);
                const gesperrt = !z.waehlbar && !bestand.includes(z.id);
                const grundId = `ablage-cloud-grund-${i}`;
                const grund = z.grund
                  ? t(`ablage.cloud.grund.${z.grund}`, {
                      regelwerk: regelwerkName,
                      defaultValue: "",
                    })
                  : "";
                return (
                  <label
                    key={z.id}
                    className="ablage-cb"
                    data-an={an}
                    data-gesperrt={gesperrt}
                  >
                    <input
                      type="checkbox"
                      checked={an}
                      disabled={gesperrt}
                      aria-describedby={grund ? grundId : undefined}
                      onChange={(e) => aendern(z.id, e.target.checked)}
                      data-testid={`ablage-cloud-anbieter-${z.id}`}
                    />
                    <span className="ablage-cb__text">
                      <b>{z.name}</b>
                      {(gesperrt || grund) && (
                        <span id={grundId} className="ablage-cb__grund">
                          {gesperrt && `${t("ablage.cloud.gesperrt")}`}
                          {gesperrt && grund && " · "}
                          {grund}
                        </span>
                      )}
                    </span>
                  </label>
                );
              })}
            </fieldset>
            {cloudFehler && (
              <p
                id="ablage-cloud-fehler"
                role="alert"
                className="ablage-fehler"
                data-testid="ablage-cloud-fehler"
              >
                {t("ablage.cloud.keinAnbieter")}
              </p>
            )}
          </>
        )}
        <button
          type="button"
          className="ablage-lnkrow"
          onClick={wechselZuRegelwerk}
          data-testid="ablage-cloud-regelwerk"
        >
          <Settings size={16} aria-hidden="true" />
          {t("ablage.cloud.verwalten")}
        </button>
      </>
    );
  };

  const bestaetigungCloud = (
    <div className="text-sm" data-testid="ablage-cloud-bestaetigung">
      <ul className="ablage-ul">
        {auswahlWirksam.length > 0 &&
          namenVon(auswahlWirksam).map((n) => (
            <li key={n}>
              <b>
                {t("ablage.cloud.anbieterZeile", {
                  name: n,
                  regelwerk: regelwerkName,
                })}
              </b>
            </li>
          ))}
      </ul>
      {zaehlung.personenbezogen.length > 0 && (
        <p className="ablage-mut" style={{ margin: "8px 0 0" }}>
          {t("ablage.cloud.ausgenommen", {
            namen: zaehlung.personenbezogen.map((n) => `„${n}“`).join(", "),
          })}
        </p>
      )}
      <div className="ablage-grp" style={{ padding: "12px 0 4px" }}>
        {t("ablage.cloud.bedeutet")}
      </div>
      <ul className="ablage-ul">
        <li>{t("ablage.cloud.punkt1")}</li>
        <li>{t("ablage.cloud.punkt2")}</li>
        <li>{t("ablage.cloud.punkt3")}</li>
        <li>{t("ablage.cloud.punkt4")}</li>
      </ul>
    </div>
  );

  return (
    <Dialog
      open={open}
      className="ablage-dlg"
      onOpenChange={(o) => !o && !busy && onClose()}
      title={titel}
      description={beschreibung}
      closeLabel={t("ablage.schliessen")}
      initialFocusRef={
        unklar || schritt !== "form" || verknuepft ? undefined : kiRef
      }
      footer={fuss}
    >
      {schritt === "nur" || schritt === "verlassen" ? null : schritt ===
        "cloud" ? (
        bestaetigungCloud
      ) : (
        <div>
          {verknuepft ? (
            <>
              <span
                className="ablage-chip"
                data-testid="ablage-einstellungen-chip"
              >
                <Eye size={14} aria-hidden="true" />
                {t("ablage.verknuepft.angabe")}
              </span>
              <div
                className="ablage-grp"
                style={{ padding: "12px 0 4px" }}
                id="ablage-ki-label"
              >
                {t("ablage.ki.fuerDieKi")}
              </div>
              <p
                className="ablage-mut"
                style={{ margin: "0 0 8px" }}
                data-testid="ablage-ki-nicht-verfuegbar"
              >
                {t("ablage.ki.verknuepftNichtVerfuegbar")}
              </p>
              <Hinweis testid="ablage-keine-ki">
                <p>{t("ablage.verknuepft.keineKi")}</p>
                <p>{t("ablage.verknuepft.keineKi2")}</p>
              </Hinweis>
            </>
          ) : (
            <>
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
            </>
          )}
          {!verknuepft && hatEltern && eigeneKi && !unklar && (
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
          {verknuepft ? (
            <p
              className="ablage-mut"
              style={{ margin: "8px 0 0" }}
              data-testid="ablage-einstellungen-verknuepft"
            >
              {t("ablage.verknuepft.einstellungenHinweis")}
            </p>
          ) : (
            <>
              <div
                className="ablage-grp"
                style={{ padding: "12px 0 4px" }}
                data-testid="ablage-cloud-abschnitt"
              >
                {t("ablage.cloud.titel")}
              </div>
              {cloudAbschnitt()}
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
            </>
          )}
        </div>
      )}
    </Dialog>
  );
};
