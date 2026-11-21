import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Checkbox } from "@fluentui/react-components";
import { ArrowLeft, Play, Square, Loader2 } from "lucide-react";
import { commands } from "@/bindings";
import { useAssistentStore } from "@/stores/assistentStore";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import type { Entwurf } from "@/stores/assistentEditStore";
import { useAssistentEditStore } from "@/stores/assistentEditStore";
import {
  istEnglisch,
  loeseBegruessung,
  type PlatzhalterWerte,
  type Profil,
  type ProfilFehler,
} from "@/components/voice-call/assistenten";
import {
  ROLLE_BUDGET_KLEIN,
  werteJetzt,
} from "@/components/voice-call/profil/wirkung";
import {
  GRENZEN,
  NACHFRAGE_STUFEN,
  NACHFRAGE_VORSCHLAG,
  PLATZHALTER,
  SPRACHEN,
  bereichSchalten,
  bereichsAnzeige,
  bereichsSumme,
  fehlerJeFeld,
  istGeaendert,
  mit,
  mitVerhalten,
  mitWissen,
  pruefe,
  rollenStand,
  zeichen,
  type FeldId,
} from "@/components/voice-call/profil/editor";
import type { EigenerAssistent } from "@/components/voice-call/profil/speicher";
import { useKnowledge } from "@/components/voice-call/useKnowledge";
import { VoicePreviewButton } from "../settings/tts/voices/VoicePreviewButton";
import { Button } from "../ui/Button";
import { Dropdown } from "../ui/Dropdown";
import { Input } from "../ui/Input";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { Textarea } from "../ui/Textarea";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { AssistentMenue } from "./AssistentMenue";
import { Aussprache } from "./Aussprache";
import { useSprachName } from "./anzeige";
import { fehlerZeile } from "./fehler";
import { useHoerprobe, useVorschauKoordination } from "./useHoerprobe";

/** Der Eintrag „Standardstimme“ der Auswahl (ein leerer Wert ließe sich in der Liste nicht als gewählt zeigen). */
const STANDARD_STIMME = "__standard__";

// ---------------------------------------------------------------- Bausteine

/** Ein Feld mit Beschriftung, Hinweis, Zähler und Fehlern; die Verbindung zum Eingabefeld (id, aria) stellt es her. */
const Feld: React.FC<{
  label: string;
  hinweis?: React.ReactNode;
  zaehler?: React.ReactNode;
  fehler?: string[];
  testId?: string;
  children: (p: {
    id: string;
    "aria-describedby": string | undefined;
    "aria-invalid": true | undefined;
  }) => React.ReactNode;
}> = ({ label, hinweis, zaehler, fehler = [], testId, children }) => {
  const id = useId();
  const hinweisId = `${id}-h`;
  const fehlerId = `${id}-f`;
  const beschreibung =
    [hinweis ? hinweisId : "", fehler.length > 0 ? fehlerId : ""]
      .filter(Boolean)
      .join(" ") || undefined;
  return (
    <div className="lv-asst-feld" data-testid={testId}>
      <div className="lv-asst-feld__zeile">
        <label htmlFor={id} className="lv-asst-feld__beschriftung">
          {label}
        </label>
        {zaehler}
      </div>
      {hinweis && (
        <p id={hinweisId} className="lv-asst-hinweis">
          {hinweis}
        </p>
      )}
      {children({
        id,
        "aria-describedby": beschreibung,
        "aria-invalid": fehler.length > 0 ? true : undefined,
      })}
      {fehler.length > 0 && (
        <div id={fehlerId}>
          {fehler.map((f) => (
            <p key={f} className="lv-asst-fehlerzeile">
              {f}
            </p>
          ))}
        </div>
      )}
    </div>
  );
};

/** Zeichenzähler „n / max“; `stand` färbt ihn bei Warnung oder Überschreitung (die Zahl steht immer auch als Text da). */
const Zaehler: React.FC<{
  n: number;
  max: number;
  stand?: "ok" | "warn" | "fehler";
  testId?: string;
}> = ({ n, max, stand = "ok", testId }) => {
  const { t } = useTranslation();
  return (
    <span className="lv-asst-zaehler" data-stand={stand} data-testid={testId}>
      {t("assistenten.zaehler", { n, max })}
    </span>
  );
};

/** Die Fehler der Prüfung zu einem Feld als Sätze (leere Liste, wenn es keine gibt). */
function saetze(
  t: TFunction,
  feld: FeldId,
  fehler: ProfilFehler[] | undefined,
): string[] {
  const max = (GRENZEN as Record<string, number>)[feld];
  return (fehler ?? []).map((f) => {
    const key = `assistenten.fehlerFeld.${feld}.${f.code}`;
    return t(key, { defaultValue: fehlerZeile(t, f), max });
  });
}

/** Eine Hörprobe als Knopf: spielt oder hält an, zeigt das Laden und einen Fehler in Worten. */
const HoerprobeKnopf: React.FC<{
  /** Der Text, der gesprochen wird (leer: keine Hörprobe möglich). */
  text: string;
  label: string;
  testId: string;
}> = ({ text, label, testId }) => {
  const { t } = useTranslation();
  const h = useHoerprobe(testId);
  const laedt = h.zustand.art === "laedt";
  const spielt = h.zustand.art === "spielt";
  const fehler = h.zustand.art === "fehler" ? h.zustand.grund : null;
  const grund = h.gespraechLaeuft ? "gespraech" : fehler;
  const meldungId = useId();
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        variant={spielt ? "primary" : "secondary"}
        size="sm"
        disabledFocusable={h.gespraechLaeuft || text.trim() === ""}
        aria-describedby={grund ? meldungId : undefined}
        onClick={() => (spielt || laedt ? h.stopp() : h.spiele(text))}
        data-testid={testId}
        type="button"
      >
        {laedt ? (
          <Loader2
            width={14}
            height={14}
            className="animate-spin"
            aria-hidden="true"
          />
        ) : spielt ? (
          <Square width={14} height={14} aria-hidden="true" />
        ) : (
          <Play width={14} height={14} aria-hidden="true" />
        )}
        {laedt
          ? t("assistenten.hoerprobe.laedt")
          : spielt
            ? t("assistenten.hoerprobe.stopp")
            : label}
      </Button>
      <span
        id={meldungId}
        className={
          grund && grund !== "leer" ? "lv-asst-fehlerzeile" : "lv-asst-hinweis"
        }
        role={fehler ? "alert" : undefined}
        data-testid={`${testId}-meldung`}
      >
        {grund ? t(`assistenten.hoerprobe.fehler.${grund}`) : ""}
      </span>
    </div>
  );
};

/** Die Stimmen dieses PC mit ihren Namen (für die Auswahl); leer, solange der Sprachdienst sie nicht nennt. */
function useStimmen(): { id: string; name: string }[] {
  const [stimmen, setStimmen] = useState<{ id: string; name: string }[]>([]);
  useEffect(() => {
    let lebt = true;
    const laden = () => {
      void Promise.resolve(commands.ttsListVoiceInfos())
        .then((infos) => {
          if (!lebt || !Array.isArray(infos)) return;
          setStimmen(
            infos.map((i) => ({ id: i.id, name: i.meta.display_name || i.id })),
          );
        })
        .catch(() => undefined);
    };
    laden();
    window.addEventListener("lv-voices-changed", laden);
    return () => {
      lebt = false;
      window.removeEventListener("lv-voices-changed", laden);
    };
  }, []);
  return stimmen;
}

// ---------------------------------------------------------------- Editor

interface Props {
  /** Der gespeicherte Stand (Profil und Text der Datei). */
  eigener: EigenerAssistent;
  entwurf: Entwurf;
  gewaehlt: boolean;
  /** Schreibende Vorgänge erlaubt (Speicher lesbar). */
  schreibt: boolean;
  /** Ein Vorgang läuft. */
  laeuft: boolean;
  /** Fehler des letzten Speicherns (Satz), sonst `null`. */
  speicherFehler: string | null;
  onSpeichern: () => void;
  onVerwerfen: () => void;
  onNeuLaden: () => void;
  onAlsKopie: () => void;
  onZurueck: () => void;
  onVerwenden: () => void;
  onKopieren: () => void;
  onUmbenennen: () => void;
  onExportieren: () => void;
  onLoeschen: () => void;
}

/**
 * Der Editor eines eigenen Assistenten: Name, Beschreibung, Anweisungen (mit Zähler gegen das Budget und Platzhaltern),
 * Begrüßung mit Hörprobe, Sprache und Stimme, Gesprächsverhalten, Wissen (das Profil begrenzt nur) und die Ausspracheregeln.
 * Der Entwurf ist ein ganzes Profil: unbekannte Felder bleiben erhalten. Gespeichert wird erst mit
 * „Speichern“; eine Eingabe außerhalb der Grenzen wird nie still gekürzt, das Feld sagt, was nicht passt.
 */
export const AssistentEditor: React.FC<Props> = ({
  eigener,
  entwurf,
  gewaehlt,
  schreibt,
  laeuft,
  speicherFehler,
  onSpeichern,
  onVerwerfen,
  onNeuLaden,
  onAlsKopie,
  onZurueck,
  onVerwenden,
  onKopieren,
  onUmbenennen,
  onExportieren,
  onLoeschen,
}) => {
  const { t } = useTranslation();
  const p = entwurf.profil;
  const aendere = useAssistentEditStore((s) => s.aendere);
  const kontoName = useAssistentStore((s) => s.name);
  const gespraechLaeuft = useVoiceCallStore((s) => s.state !== "idle");
  const { areas: freigegeben } = useKnowledge();
  const stimmen = useStimmen();
  const vorschau = useVorschauKoordination();
  const anweisungenRef = useRef<HTMLTextAreaElement>(null);
  const [werte, setWerte] = useState<PlatzhalterWerte>(() =>
    werteJetzt(kontoName),
  );
  // Die Werte für die Vorschau: heute, mit dem Namen aus dem Konto (neu bei jedem Öffnen).
  useEffect(() => setWerte(werteJetzt(kontoName)), [kontoName, entwurf.id]);

  const fehler = useMemo(() => pruefe(p), [p]);
  const jeFeld = useMemo(() => fehlerJeFeld(fehler), [fehler]);
  const geaendert = useMemo(
    () => istGeaendert(entwurf.basis.profil, p),
    [entwurf.basis.profil, p],
  );
  const stand = useMemo(() => rollenStand(p, werte), [p, werte]);
  const bereiche = useMemo(
    () => bereichsAnzeige(p, freigegeben),
    [p, freigegeben],
  );
  const summe = useMemo(() => bereichsSumme(p, freigegeben), [p, freigegeben]);
  // Fehler zu Feldern ohne eigene Stelle im Editor (Sprache, Stimme, Wissen, unbekannte Felder): nichts bleibt unsichtbar.
  const restFehler = useMemo(
    () =>
      (["sprache", "stimme", "wissen", "sonst"] as const).flatMap((f) =>
        saetze(t, f, jeFeld[f]),
      ),
    [jeFeld, t],
  );
  const konflikt = entwurf.konflikt;
  const verwendenGrundId = useId();
  // „Im Gespräch verwenden“ nimmt den gespeicherten Stand und beginnt ein neues Gespräch: ein laufendes Gespräch endet nicht
  // nebenbei, und ungespeicherte Änderungen gelten nicht, ohne dass jemand es merkt.
  const verwendenGrund = !schreibt
    ? t("assistenten.kopf.verwendenGesperrt")
    : gespraechLaeuft
      ? t("assistenten.kopf.verwendenGespraech")
      : geaendert
        ? t("assistenten.kopf.verwendenUngespeichert")
        : null;
  const kannSpeichern =
    schreibt &&
    geaendert &&
    fehler.length === 0 &&
    konflikt === null &&
    !laeuft;
  const begruessung = loeseBegruessung(p, werte).text;
  const sprachName = useSprachName();

  const setze = (aenderung: (profil: Profil) => Profil) => aendere(aenderung);

  const platzhalterEinfuegen = (name: string) => {
    const feld = anweisungenRef.current;
    const marke = `{{${name}}}`;
    const von = feld?.selectionStart ?? p.anweisungen.length;
    const bis = feld?.selectionEnd ?? von;
    const neu = p.anweisungen.slice(0, von) + marke + p.anweisungen.slice(bis);
    setze((x) => mit(x, { anweisungen: neu }));
    requestAnimationFrame(() => {
      const f = anweisungenRef.current;
      if (!f) return;
      f.focus();
      f.setSelectionRange(von + marke.length, von + marke.length);
    });
  };

  const sprachOptionen = useMemo(() => {
    const liste = SPRACHEN.includes(p.sprache)
      ? [...SPRACHEN]
      : [p.sprache, ...SPRACHEN];
    return liste.map((code) => ({ value: code, label: sprachName(code) }));
  }, [p.sprache, sprachName]);

  const stimmenOptionen = useMemo(() => {
    const standard = {
      value: STANDARD_STIMME,
      label: t("assistenten.stimme.standard"),
    };
    const bekannt = stimmen.map((s) => ({ value: s.id, label: s.name }));
    const fremd =
      p.stimme !== null && !stimmen.some((s) => s.id === p.stimme)
        ? [
            {
              value: p.stimme,
              label: t("assistenten.stimme.nichtVorhanden", { name: p.stimme }),
            },
          ]
        : [];
    return [standard, ...bekannt, ...fremd];
  }, [stimmen, p.stimme, t]);
  const stimmeVorhanden =
    p.stimme !== null && stimmen.some((s) => s.id === p.stimme);
  const probeText = istEnglisch(p.sprache)
    ? t("assistenten.hoerprobe.beispiel", { lng: "en" })
    : t("assistenten.hoerprobe.beispiel", { lng: "de" });

  const nachfrageOptionen = useMemo(() => {
    const stufen = NACHFRAGE_STUFEN.includes(p.verhalten.nachfrageStilleS)
      ? [...NACHFRAGE_STUFEN]
      : [...NACHFRAGE_STUFEN, p.verhalten.nachfrageStilleS].sort(
          (a, b) => a - b,
        );
    return stufen.map((n) => ({
      value: String(n),
      label:
        n === 0
          ? t("assistenten.verhalten.nachfrage.aus")
          : t("assistenten.verhalten.nachfrage.sekunden", { count: n }),
    }));
  }, [p.verhalten.nachfrageStilleS, t]);

  const anweisungenStand: "ok" | "warn" | "fehler" = stand.ueberMax
    ? "fehler"
    : stand.ueberKlein
      ? "warn"
      : "ok";

  return (
    <article
      className="space-y-4"
      aria-labelledby="asst-editor-titel"
      data-testid="assistent-editor"
    >
      <header className="lv-asst-kopf">
        <Button
          variant="ghost"
          size="sm"
          className="lv-asst__zurueck"
          onClick={onZurueck}
          type="button"
          data-testid="assistent-zurueck"
        >
          <ArrowLeft width={16} height={16} aria-hidden="true" />
          {t("assistenten.zurueck")}
        </Button>
        <div className="lv-asst-kopf__titel">
          <h2 id="asst-editor-titel" data-testid="assistent-editor-titel">
            {eigener.profil.name}
          </h2>
          <p>
            {gewaehlt
              ? t("assistenten.kopf.gewaehlt")
              : t("assistenten.kopf.eigen")}
          </p>
          {verwendenGrund && (
            <p
              id={verwendenGrundId}
              className="lv-asst-hinweis"
              data-testid="assistent-verwenden-grund"
            >
              {verwendenGrund}
            </p>
          )}
        </div>
        <div className="lv-asst-kopf__aktionen">
          <Button
            variant="secondary"
            size="sm"
            type="button"
            onClick={onVerwenden}
            disabledFocusable={verwendenGrund !== null}
            aria-describedby={verwendenGrund ? verwendenGrundId : undefined}
            data-testid="assistent-verwenden"
          >
            {t("assistenten.aktion.verwenden")}
          </Button>
          <AssistentMenue
            name={eigener.profil.name}
            art="eigen"
            schreibt={schreibt && !laeuft}
            onKopieren={onKopieren}
            onUmbenennen={onUmbenennen}
            onExportieren={onExportieren}
            onLoeschen={onLoeschen}
            testId="assistent-editor-menue"
            className="lv-btn lv-asst-zeile__menue"
          />
        </div>
      </header>

      {konflikt && (
        <div
          className="lv-asst-meldung"
          data-art="fehler"
          role="alert"
          data-testid="assistent-konflikt"
        >
          <h3>
            {konflikt === "fehlt"
              ? t("assistenten.konflikt.fehltTitel")
              : t("assistenten.konflikt.titel")}
          </h3>
          <p className="m-0">
            {konflikt === "fehlt"
              ? t("assistenten.konflikt.fehltText")
              : t("assistenten.konflikt.text")}
          </p>
          <div className="flex flex-wrap gap-2 pt-1">
            <Button
              variant="primary"
              size="sm"
              type="button"
              onClick={onAlsKopie}
              disabled={!schreibt || laeuft}
              data-testid="assistent-konflikt-kopie"
            >
              {t("assistenten.konflikt.alsKopie")}
            </Button>
            {konflikt === "geaendert" && (
              <Button
                variant="secondary"
                size="sm"
                type="button"
                onClick={onNeuLaden}
                data-testid="assistent-konflikt-neuladen"
              >
                {t("assistenten.konflikt.neuLaden")}
              </Button>
            )}
          </div>
        </div>
      )}

      <div className="lv-asst-abschnitte">
        {/* ------------------------------------------------ Allgemein */}
        <SettingsGroup title={t("assistenten.abschnitt.allgemein")}>
          <div className="lv-asst-block">
            <Feld
              label={t("assistenten.name.label")}
              zaehler={
                <Zaehler
                  n={zeichen(p.name.trim())}
                  max={GRENZEN.name}
                  stand={jeFeld.name ? "fehler" : "ok"}
                />
              }
              fehler={saetze(t, "name", jeFeld.name)}
              testId="assistent-feld-name"
            >
              {(a) => (
                <Input
                  {...a}
                  value={p.name}
                  onChange={(e) =>
                    setze((x) => mit(x, { name: e.target.value }))
                  }
                  autoComplete="off"
                  data-testid="assistent-name"
                />
              )}
            </Feld>
            <Feld
              label={t("assistenten.beschreibung.label")}
              hinweis={t("assistenten.beschreibung.hinweis")}
              zaehler={
                <Zaehler
                  n={zeichen(p.beschreibung)}
                  max={GRENZEN.beschreibung}
                  stand={jeFeld.beschreibung ? "fehler" : "ok"}
                />
              }
              fehler={saetze(t, "beschreibung", jeFeld.beschreibung)}
              testId="assistent-feld-beschreibung"
            >
              {(a) => (
                <Textarea
                  {...a}
                  rows={2}
                  value={p.beschreibung}
                  onChange={(e) =>
                    setze((x) => mit(x, { beschreibung: e.target.value }))
                  }
                  data-testid="assistent-beschreibung"
                />
              )}
            </Feld>
          </div>
        </SettingsGroup>

        {/* ------------------------------------------------ Anweisungen */}
        <SettingsGroup title={t("assistenten.abschnitt.anweisungen")}>
          <div className="lv-asst-block">
            <Feld
              label={t("assistenten.anweisungen.label")}
              hinweis={t("assistenten.anweisungen.hinweis", {
                klein: ROLLE_BUDGET_KLEIN,
              })}
              zaehler={
                <Zaehler
                  n={stand.zeichen}
                  max={GRENZEN.anweisungen}
                  stand={anweisungenStand}
                  testId="assistent-anweisungen-zaehler"
                />
              }
              fehler={saetze(t, "anweisungen", jeFeld.anweisungen)}
              testId="assistent-feld-anweisungen"
            >
              {(a) => (
                <Textarea
                  {...a}
                  ref={anweisungenRef}
                  rows={10}
                  value={p.anweisungen}
                  onChange={(e) =>
                    setze((x) => mit(x, { anweisungen: e.target.value }))
                  }
                  data-testid="assistent-anweisungen"
                />
              )}
            </Feld>
            {stand.ueberKlein && !stand.ueberMax && (
              <p
                className="lv-asst-hinweis"
                data-testid="assistent-budget-hinweis"
              >
                {t("assistenten.anweisungen.ueberKlein", {
                  klein: ROLLE_BUDGET_KLEIN,
                })}
              </p>
            )}
            {stand.unbekannt.length > 0 && (
              <p
                className="lv-asst-hinweis"
                data-testid="assistent-platzhalter-unbekannt"
              >
                {t("assistenten.platzhalter.unbekannt", {
                  namen: stand.unbekannt.map((n) => `{{${n}}}`).join(", "),
                })}
              </p>
            )}
            <div
              role="group"
              aria-label={t("assistenten.platzhalter.gruppe")}
              className="space-y-1"
            >
              <p className="lv-asst-hinweis">
                {t("assistenten.platzhalter.hilfe")}
              </p>
              <div className="lv-asst-platzhalter">
                {PLATZHALTER.map((n) => (
                  <Button
                    key={n}
                    variant="secondary"
                    size="sm"
                    type="button"
                    onClick={() => platzhalterEinfuegen(n)}
                    aria-label={t("assistenten.platzhalter.einfuegen", {
                      name: t(`assistenten.platzhalter.namen.${n}`),
                    })}
                    data-testid={`assistent-platzhalter-${n}`}
                  >
                    <code>{`{{${n}}}`}</code>
                  </Button>
                ))}
              </div>
            </div>
          </div>
        </SettingsGroup>

        {/* ------------------------------------------------ Begrüßung */}
        <SettingsGroup title={t("assistenten.abschnitt.begruessung")}>
          <div className="lv-asst-block">
            <Feld
              label={t("assistenten.begruessung.label")}
              hinweis={t("assistenten.begruessung.hinweis")}
              zaehler={
                <Zaehler
                  n={zeichen(p.begruessung)}
                  max={GRENZEN.begruessung}
                  stand={jeFeld.begruessung ? "fehler" : "ok"}
                />
              }
              fehler={saetze(t, "begruessung", jeFeld.begruessung)}
              testId="assistent-feld-begruessung"
            >
              {(a) => (
                <Textarea
                  {...a}
                  rows={3}
                  value={p.begruessung}
                  onChange={(e) =>
                    setze((x) => mit(x, { begruessung: e.target.value }))
                  }
                  data-testid="assistent-begruessung"
                />
              )}
            </Feld>
            <div className="space-y-1">
              <p className="lv-asst-hinweis">
                {t("assistenten.begruessung.vorschau")}
              </p>
              <p
                className="lv-asst-vorschau"
                data-testid="assistent-begruessung-vorschau"
              >
                {begruessung || t("assistenten.begruessung.keine")}
              </p>
              {kontoName === null && p.begruessung.includes("{{name}}") && (
                <p className="lv-asst-hinweis">
                  {t("assistenten.begruessung.ohneName")}
                </p>
              )}
            </div>
            <HoerprobeKnopf
              text={begruessung}
              label={t("assistenten.begruessung.anhoeren")}
              testId="assistent-begruessung-hoeren"
            />
            <p className="lv-asst-hinweis">
              {t("assistenten.begruessung.hoerHinweis")}
            </p>
          </div>
        </SettingsGroup>

        {/* ------------------------------------------------ Sprache und Stimme */}
        <SettingsGroup
          title={t("assistenten.abschnitt.stimme")}
          description={t("assistenten.stimme.ohneWirkung")}
        >
          <SettingContainer
            title={t("assistenten.sprache.label")}
            description={t("assistenten.sprache.hinweis")}
            descriptionMode="inline"
            grouped
          >
            <Dropdown
              options={sprachOptionen}
              selectedValue={p.sprache}
              onSelect={(v) => setze((x) => mit(x, { sprache: v }))}
              ariaLabel={t("assistenten.sprache.label")}
            />
          </SettingContainer>
          <SettingContainer
            title={t("assistenten.stimme.label")}
            description={t("assistenten.stimme.hinweis")}
            descriptionMode="inline"
            grouped
          >
            <Dropdown
              options={stimmenOptionen}
              selectedValue={p.stimme ?? STANDARD_STIMME}
              onSelect={(v) =>
                setze((x) =>
                  mit(x, { stimme: v === STANDARD_STIMME ? null : v }),
                )
              }
              ariaLabel={t("assistenten.stimme.label")}
            />
          </SettingContainer>
          <div className="lv-asst-block">
            {stimmeVorhanden && p.stimme ? (
              <div className="flex flex-wrap items-center gap-2">
                <VoicePreviewButton
                  voiceId={p.stimme}
                  gesperrt={vorschau.gesperrt}
                  beiStart={vorschau.beiStart}
                />
                <span
                  className={
                    vorschau.gesperrt
                      ? "lv-asst-fehlerzeile"
                      : "lv-asst-hinweis"
                  }
                  data-testid="assistent-stimme-vorschau-meldung"
                >
                  {vorschau.gesperrt
                    ? t("assistenten.hoerprobe.fehler.gespraech")
                    : t("assistenten.stimme.probeStimme")}
                </span>
              </div>
            ) : (
              <>
                <HoerprobeKnopf
                  text={probeText}
                  label={t("assistenten.stimme.anhoeren")}
                  testId="assistent-stimme-hoeren"
                />
                <p className="lv-asst-hinweis">
                  {t("assistenten.stimme.probeStandard")}
                </p>
              </>
            )}
          </div>
        </SettingsGroup>

        {/* ------------------------------------------------ Gesprächsverhalten */}
        <SettingsGroup title={t("assistenten.abschnitt.verhalten")}>
          <SettingContainer
            title={t("assistenten.verhalten.stil.label")}
            description={`${t("assistenten.verhalten.stil.hinweis")} ${t("assistenten.verhalten.ohneWirkung")}`}
            descriptionMode="inline"
            grouped
          >
            <Dropdown
              options={[
                {
                  value: "natuerlich",
                  label: t("assistenten.verhalten.stil.natuerlich"),
                },
                {
                  value: "woertlich",
                  label: t("assistenten.verhalten.stil.woertlich"),
                },
              ]}
              selectedValue={p.verhalten.stil}
              onSelect={(v) =>
                setze((x) =>
                  mitVerhalten(x, { stil: v as Profil["verhalten"]["stil"] }),
                )
              }
              ariaLabel={t("assistenten.verhalten.stil.label")}
            />
          </SettingContainer>
          <SettingContainer
            title={t("assistenten.verhalten.pause.label")}
            description={t("assistenten.verhalten.pause.hinweis")}
            descriptionMode="inline"
            grouped
          >
            <Dropdown
              options={[
                { value: "kurz", label: t("assistenten.verhalten.pause.kurz") },
                {
                  value: "normal",
                  label: t("assistenten.verhalten.pause.normal"),
                },
                { value: "lang", label: t("assistenten.verhalten.pause.lang") },
              ]}
              selectedValue={p.verhalten.sprechpause}
              onSelect={(v) =>
                setze((x) =>
                  mitVerhalten(x, {
                    sprechpause: v as Profil["verhalten"]["sprechpause"],
                  }),
                )
              }
              ariaLabel={t("assistenten.verhalten.pause.label")}
            />
          </SettingContainer>
          <ToggleSwitch
            checked={p.verhalten.unterbrechen}
            onChange={(an) =>
              setze((x) => mitVerhalten(x, { unterbrechen: an }))
            }
            label={t("assistenten.verhalten.unterbrechen.label")}
            description={t("assistenten.verhalten.unterbrechen.hinweis")}
            descriptionMode="inline"
            grouped
            testId="assistent-unterbrechen"
          />
          <SettingContainer
            title={t("assistenten.verhalten.nachfrage.label")}
            description={`${t("assistenten.verhalten.nachfrage.hinweis", {
              vorschlag: NACHFRAGE_VORSCHLAG,
            })} ${t("assistenten.verhalten.ohneWirkung")}`}
            descriptionMode="inline"
            grouped
          >
            <Dropdown
              options={nachfrageOptionen}
              selectedValue={String(p.verhalten.nachfrageStilleS)}
              onSelect={(v) =>
                setze((x) => mitVerhalten(x, { nachfrageStilleS: Number(v) }))
              }
              ariaLabel={t("assistenten.verhalten.nachfrage.label")}
            />
          </SettingContainer>
          {jeFeld.verhalten && (
            <div className="lv-asst-block">
              {saetze(t, "verhalten", jeFeld.verhalten).map((s) => (
                <p key={s} className="lv-asst-fehlerzeile">
                  {s}
                </p>
              ))}
            </div>
          )}
        </SettingsGroup>

        {/* ------------------------------------------------ Wissen */}
        <SettingsGroup
          title={t("assistenten.abschnitt.wissen")}
          description={t("assistenten.wissen.regel")}
        >
          <ToggleSwitch
            checked={p.wissen.alleBereiche}
            onChange={(an) => setze((x) => mitWissen(x, { alleBereiche: an }))}
            label={t("assistenten.wissen.alle.label")}
            description={t("assistenten.wissen.alle.hinweis")}
            descriptionMode="inline"
            grouped
            testId="assistent-wissen-alle"
          />
          <div
            className="lv-asst-block"
            role="group"
            aria-label={t("assistenten.wissen.gruppe")}
          >
            {bereiche.map((z) => {
              const name = t(`assistenten.wissen.bereich.${z.schluessel}`, {
                defaultValue: z.schluessel,
              });
              const status =
                z.pc === null
                  ? t("assistenten.wissen.status.ohneWirkung")
                  : !z.freigegeben
                    ? t("assistenten.wissen.status.nichtFreigegeben")
                    : z.gewollt
                      ? t("assistenten.wissen.status.wirkt")
                      : t("assistenten.wissen.status.freigegeben");
              return (
                <label
                  key={z.schluessel}
                  className="lv-asst-bereich"
                  data-testid={`assistent-bereich-${z.schluessel}`}
                >
                  <Checkbox
                    checked={z.gewollt}
                    disabled={p.wissen.alleBereiche}
                    onChange={(_e, d) =>
                      setze((x) =>
                        bereichSchalten(x, z.schluessel, d.checked === true),
                      )
                    }
                    aria-label={name}
                    data-testid={`assistent-bereich-schalter-${z.schluessel}`}
                  />
                  <span className="lv-asst-bereich__text">{name}</span>
                  <span
                    className="lv-asst-bereich__status"
                    data-wirkt={z.wirksam ? "ja" : "nein"}
                  >
                    {status}
                  </span>
                </label>
              );
            })}
            <p
              className="lv-asst-hinweis"
              role="status"
              data-testid="assistent-wissen-summe"
            >
              {t("assistenten.wissen.summe", {
                wirksam: summe.wirksam,
                frei: summe.freigegeben,
              })}
            </p>
            {p.wissen.kontexte.length > 0 && (
              <p
                className="lv-asst-hinweis"
                data-testid="assistent-wissen-kontexte"
              >
                {t("assistenten.wissen.kontexte", {
                  count: p.wissen.kontexte.length,
                })}
              </p>
            )}
          </div>
        </SettingsGroup>

        {/* ------------------------------------------------ Aussprache */}
        <Aussprache profil={p} fehler={fehler} onAendern={setze} />
      </div>

      {restFehler.length > 0 && (
        <div
          className="lv-asst-meldung"
          data-art="fehler"
          role="alert"
          data-testid="assistent-restfehler"
        >
          <h3>{t("assistenten.restfehler.titel")}</h3>
          <ul>
            {restFehler.map((z) => (
              <li key={z}>{z}</li>
            ))}
          </ul>
        </div>
      )}

      {/* ------------------------------------------------ Leiste */}
      <div
        className="lv-asst-leiste"
        role="group"
        aria-label={t("assistenten.leiste.label")}
      >
        <div
          className="lv-asst-leiste__status"
          data-stand={geaendert ? "ungespeichert" : "gespeichert"}
          role="status"
          data-testid="assistent-status"
        >
          {speicherFehler ? (
            <span
              className="lv-asst-fehlerzeile"
              role="alert"
              data-testid="assistent-speicherfehler"
            >
              {speicherFehler}
            </span>
          ) : geaendert ? (
            t("assistenten.leiste.ungespeichert")
          ) : (
            t("assistenten.leiste.gespeichert")
          )}
          {gewaehlt && (
            <span className="lv-asst-hinweis block">
              {t("assistenten.leiste.abNaechstem")}
            </span>
          )}
          {fehler.length > 0 && (
            <span className="lv-asst-fehlerzeile block">
              {t("assistenten.leiste.fehler", { count: fehler.length })}
            </span>
          )}
        </div>
        <Button
          variant="primary"
          type="button"
          onClick={onSpeichern}
          disabledFocusable={!kannSpeichern}
          data-testid="assistent-speichern"
        >
          {t("assistenten.aktion.speichern")}
        </Button>
        <Button
          variant="secondary"
          type="button"
          onClick={onVerwerfen}
          disabled={!geaendert || laeuft}
          data-testid="assistent-verwerfen"
        >
          {t("assistenten.aktion.verwerfen")}
        </Button>
      </div>
    </article>
  );
};
